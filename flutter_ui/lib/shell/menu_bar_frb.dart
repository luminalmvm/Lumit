// The menu bar: nine menus in the After Effects arrangement (K-244).
//
// **One tree, two renderers.** [lumitMenus] returns the whole bar as data —
// labels, shortcuts, enablement, ticks — and nothing in it knows how a menu is
// drawn. Windows and Linux get the in-app bar at the bottom of this file;
// macOS hands the same tree to the operating system through `PlatformMenuBar`,
// so Lumit's menus live in the Mac menu bar where every other Mac app's do,
// with Settings and About in the application menu as Apple's guidelines ask.
// Neither renderer holds a list of its own, which is the only way the two
// cannot drift apart.
//
// **What a row can say.** Every engine-backed item calls straight through a
// reference handle. An item with no action yet is still *listed*, marked
// "(Not implemented)" and disabled, so the shape of the finished application is
// visible while it is being built and nobody has to guess whether a command is
// missing or broken. An item whose command needs something that is not there —
// no project, no composition, no selected layer — greys out rather than failing
// when pressed: an item you can see is disabled tells you the state of the
// document, where one that does nothing when pressed does not.
//
// **Shortcuts are the engine's** (K-199). A row shows whatever chord the keymap
// currently binds to its action id, so rebinding in Settings ▸ Keymap changes
// the menus too, and a row whose action has no binding simply shows nothing.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/beats.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../panels/layer_fold_frb.dart' show RevealFilter;
import '../panels/timeline_extras_frb.dart';
import '../panels/timeline_group_row_frb.dart';
import '../panels/viewer_panel_frb.dart' show captureViewerPicturePng;
import '../state/clipboard.dart';
import '../state/dock.dart';
import '../state/external_links.dart';
import '../state/file_dialogs.dart';
import '../state/keymap.dart';
import '../state/viewer_view.dart';
import '../state/workspace.dart' show UserWorkspace, Workspace;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'about_window_frb.dart';
import 'ae_report_frb.dart';
import 'command_palette_frb.dart';
import 'comp_settings_frb.dart';
import 'fx_console_context.dart';
import 'fx_console_frb.dart';
import 'history_dialog_frb.dart';
import 'layer_settings_frb.dart';
import 'menu_animation_frb.dart';
import 'menu_layer_frb.dart';
import 'precompose_dialog_frb.dart';
import 'export_dialog_frb.dart';
import 'export_queue_frb.dart';
import 'recovery_dialog_frb.dart';
import 'project_settings_frb.dart';
import 'settings_window_frb.dart';
import 'theme_name_dialog.dart';
import 'update_dialog_frb.dart';
import 'workspace_shortcut_frb.dart';

/// The machinery half of Window ▸ Workspace (docs/07 §1.4): saving the
/// arrangement on screen under a name of the user's own, and renaming,
/// deleting, exporting and importing what they have saved.
///
/// The four that act on a saved workspace are disabled unless one is in force —
/// a preset's factory layout is not the user's to rename or delete. Every
/// outcome is said in the status line rather than in a dialogue of its own:
/// these are quiet acts, and the strip shows the result.
List<MenuEntry> _userWorkspaceRows(
  BuildContext context,
  LumitState app,
  LumitUiState ui,
) {
  final workspace = ui.workspace;
  final active = workspace.activeUserWorkspace;

  Future<void> saveAs() async {
    // The theme's name dialogue, because a name is a name: one small field
    // with a suggestion in it, and blank reads as cancel.
    final asked = await askThemeName(context,
        title: l10n.workspaceSaveAsTitle, suggested: l10n.workspaceNewName);
    if (asked == null) return;
    final wanted = asked.trim();
    final name = workspace.saveWorkspaceAs(wanted);
    app.postNotice(name == wanted
        ? l10n.workspaceSaved(name)
        : l10n.workspaceNameTaken(wanted, name));
  }

  Future<void> rename() async {
    if (active == null) return;
    final asked = await askThemeName(context,
        title: l10n.workspaceRenameTitle,
        suggested: active,
        confirm: l10n.rename);
    if (asked == null) return;
    final now = workspace.renameUserWorkspace(active, asked);
    if (now == null || now == active) return;
    app.postNotice(now == asked.trim()
        ? l10n.workspaceRenamedTo(now)
        : l10n.workspaceNameTaken(asked.trim(), now));
  }

  Future<void> export() async {
    if (active == null) return;
    final saved = workspace.userWorkspaces.firstWhere((w) => w.name == active);
    final path = await pickWorkspaceSaveLocation(
        Workspace.userWorkspaceFile(active).uri.pathSegments.last);
    if (path == null) return;
    try {
      await File(path).writeAsString(saved.encode());
      app.postNotice(l10n.workspaceExported(active));
    } catch (_) {
      app.postNotice(l10n.workspaceFileUnwritable, error: true);
    }
  }

  Future<void> import() async {
    final path = await pickWorkspaceToOpen();
    if (path == null) return;
    UserWorkspace? read;
    try {
      read =
          UserWorkspace.fromJson(jsonDecode(await File(path).readAsString()));
    } catch (_) {
      read = null;
    }
    if (read == null) {
      app.postNotice(l10n.workspaceFileNotAWorkspace, error: true);
      return;
    }
    final wanted = read.name;
    final name = workspace.importUserWorkspace(read);
    app.postNotice(name == wanted
        ? l10n.workspaceImported(name)
        : l10n.workspaceNameTaken(wanted, name));
  }

  return [
    MenuEntry(l10n.menuSaveWorkspaceAs, saveAs),
    MenuEntry(l10n.menuRenameWorkspace, active == null ? null : rename),
    MenuEntry(
        l10n.menuDeleteWorkspace,
        active == null
            ? null
            : () {
                workspace.deleteUserWorkspace(active);
                app.postNotice(l10n.workspaceDeleted(active));
              }),
    MenuEntry(l10n.menuExportWorkspace, active == null ? null : export),
    MenuEntry(l10n.menuImportWorkspace, import),
  ];
}

/// One row of a menu: a label with an action, a submenu, or a divider.
///
/// [action] is a keymap action id, not a callback — the chord beside the row is
/// looked up from the live keymap when the bar is built. [todo] marks a command
/// that is specified but not built; it draws disabled with "(Not implemented)"
/// after its name. [checked] makes the row a toggle, drawn with a tick column.
class MenuEntry {
  final String? label;
  final VoidCallback? onPressed;
  final List<MenuEntry>? children;
  final bool isDivider;
  final String? action;
  final bool todo;

  /// The tick a fixed row was built with. Null on a row that is not a toggle,
  /// and null on a [MenuEntry.toggle], which reads its own tick live.
  final bool? _checked;

  /// A toggle's tick, read every time the row is drawn. See [MenuEntry.toggle].
  final bool Function()? _checkedNow;

  /// What this row watches, for the few rows whose wording changes while the
  /// menu is open. Null for every ordinary row — see [MenuEntry.live].
  final Listenable? live;

  /// How to rebuild this row when [live] fires. Null unless [live] is set.
  final MenuEntry Function()? rebuild;

  MenuEntry(
    this.label,
    this.onPressed, {
    this.action,
    bool? checked,
  })  : isDivider = false,
        children = null,
        todo = false,
        _checked = checked,
        _checkedNow = null,
        live = null,
        rebuild = null;

  /// **A checkbox row that leaves the menu open** (K-520).
  ///
  /// The panel toggles in Window are used several at a time — turn Scopes on,
  /// turn Node preview off, turn Hierarchy on — and a menu that shut after
  /// each tick meant opening it again for every one of them. So a toggle row
  /// stays put: it runs [onPressed], re-reads [checked] and redraws its own
  /// tick, and the menu closes on Escape, a click away, or any row that is not
  /// a toggle, exactly as it always did.
  ///
  /// Only for genuine on/off rows. A row that *picks* one of several — a
  /// workspace preset, a preview resolution — is an ordinary [MenuEntry] with
  /// `checked:`, and closing after the choice is what a choice should do.
  MenuEntry.toggle(this.label, this.onPressed,
      {this.action, required bool Function() checked})
      : isDivider = false,
        children = null,
        todo = false,
        _checked = null,
        _checkedNow = checked,
        live = null,
        rebuild = null;

  /// **An option row that leaves the menu open** (K-671), on the same terms as
  /// [MenuEntry.toggle] and for a narrower reason: the rows that pick one of
  /// several *and* change the picture in front of you — the preview
  /// resolution. Picking one is nearly always comparing it with the last, and
  /// a menu that shut after each choice made comparing two tiers a matter of
  /// reopening the menu between every look.
  ///
  /// K-520 left these as ordinary rows because "closing is what a choice
  /// should do"; that holds for a choice you cannot see the result of without
  /// the menu out of the way — a workspace preset — and not for one whose
  /// whole point is on screen behind the menu.
  MenuEntry.option(this.label, this.onPressed,
      {this.action, required bool Function() checked})
      : isDivider = false,
        children = null,
        todo = false,
        _checked = null,
        _checkedNow = checked,
        live = null,
        rebuild = null;

  MenuEntry.divider()
      : label = null,
        onPressed = null,
        children = null,
        isDivider = true,
        action = null,
        todo = false,
        _checked = null,
        _checkedNow = null,
        live = null,
        rebuild = null;

  MenuEntry.submenu(this.label, this.children)
      : onPressed = null,
        isDivider = false,
        action = null,
        todo = false,
        _checked = null,
        _checkedNow = null,
        live = null,
        rebuild = null;

  /// A command the specification has and the build has not.
  MenuEntry.todo(this.label, {this.action})
      : onPressed = null,
        children = null,
        isDivider = false,
        todo = true,
        _checked = null,
        _checkedNow = null,
        live = null,
        rebuild = null;

  /// A row that redraws itself while its menu is open, from [rebuild], every
  /// time [live] fires — and which does *not* close the menu when pressed.
  ///
  /// Only for rows where the press has visible consequences in the row itself:
  /// Check for updates is the one, and it is the whole reason this exists
  /// (K-296). Pressing it starts a check, the row greys and says so, and the
  /// answer arrives in the same row a second or two later. A row that closed
  /// the menu would leave the user pressing Help again to find out what
  /// happened, and one that did not redraw would still say "Check for updates"
  /// while it was checking.
  MenuEntry.live(Listenable this.live, MenuEntry Function() this.rebuild)
      : label = null,
        onPressed = null,
        children = null,
        isDivider = false,
        action = null,
        todo = false,
        _checked = null,
        _checkedNow = null;

  /// The tick this row wears now: fixed for an ordinary row, re-read for a
  /// [MenuEntry.toggle] whose menu is still open after it was pressed.
  bool? get checked {
    final now = _checkedNow;
    return now == null ? _checked : now();
  }

  /// Whether pressing this row leaves the menu up — a [MenuEntry.toggle]
  /// (K-520) or a [MenuEntry.option] (K-671). Both read their tick through a
  /// closure, which is what lets it be redrawn after the press.
  bool get keepsMenuOpen => _checkedNow != null;

  /// This row as it currently reads. The same row for everything except a
  /// [MenuEntry.live] one, which asks its builder.
  MenuEntry get current => rebuild == null ? this : rebuild!();

  /// What the row reads as, suffix and all.
  String get text => todo ? l10n.notImplemented(label ?? '') : (label ?? '');

  /// Whether pressing this row does anything. A submenu is never "pressed" but
  /// is still live, so it counts as enabled when it has children.
  bool get enabled => onPressed != null || (children?.isNotEmpty ?? false);
}

/// One top-level menu: its heading, and its rows **built when it opens**.
///
/// The rows are a closure rather than a list because the bar is rebuilt by
/// things that have nothing to do with what is on screen in it — every layer
/// click, for one, since a row's enabled state reads the selection. Building
/// them eagerly there cost **12.9–15.5 ms of the click**, most of it the Effect
/// menu's entry per effect in the catalogue, for rows nobody was looking at
/// (docs/impl/ui-performance.md §4.4, WP-2). A closed menu now costs the record
/// that holds it; an open one costs what it always did, on a deliberate press.
typedef MenuSection = ({String title, List<MenuEntry> Function() items});

class LumitMenuBarFrb extends StatelessWidget {
  final LumitState app;

  /// File-picker seams. Defaulted to the real dialogues; a test injects its own,
  /// because a plugin channel cannot open in a widget test.
  final Future<String?> Function()? openPicker;
  final Future<String?> Function()? savePicker;
  final Future<List<String>> Function()? footagePicker;

  /// The After Effects project chooser — a file picker for the `.aep` itself
  /// or a zipped bundle (K-418).
  final Future<String?> Function()? aeProjectPicker;

  /// The Bridge bundle chooser — a folder picker (docs/11 §2.1).

  const LumitMenuBarFrb({
    super.key,
    required this.app,
    this.openPicker,
    this.savePicker,
    this.footagePicker,
    this.aeProjectPicker,
  });

  @override
  Widget build(BuildContext context) {
    // Half this bar's enablement is about the *selection* — the Effect menu,
    // Delete, Duplicate, Pre-compose, Retime — and the selection lives in a
    // ValueNotifier that does not notify the shell state. Without this the bar
    // would keep whatever selection it was last built with, and every one of
    // those rows would be greyed out with a layer plainly selected.
    // The updater (K-296) is watched on macOS only, where the whole tree is
    // handed to the system and there is no rebuilding a single row. In-app,
    // the Help menu's live row listens for itself (see [_MenuList]), so
    // download progress never rebuilds the whole bar.
    final ui = context.read<LumitUiState>();
    return ValueListenableBuilder<List<LayerReference>>(
      valueListenable: ui.selectedLayers,
      builder: (context, _, __) => defaultTargetPlatform == TargetPlatform.macOS
          ? ListenableBuilder(
              listenable: ui.updates,
              builder: (context, _) => _bar(context),
            )
          : _bar(context),
    );
  }

  Widget _bar(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final menus = lumitMenus(
      context,
      app,
      openPicker: openPicker,
      savePicker: savePicker,
      footagePicker: footagePicker,
      aeProjectPicker: aeProjectPicker,
      palette: () => _palette(context),
    );

    // macOS puts menus in the system bar, not in the window (K-244). The bar
    // itself draws nothing here; the hotkey holder still has to be in the tree.
    if (defaultTargetPlatform == TargetPlatform.macOS) {
      return PlatformMenuBar(
        menus: platformMenusFor(context, menus),
        child: Stack(children: [
          _RequestHotkey(
            requests: context.read<LumitUiState>().paletteRequest,
            onRequested: () => _palette(context),
          ),
          _RequestHotkey(
            requests: context.read<LumitUiState>().consoleRequest,
            onRequested: () => _console(context),
          ),
        ]),
      );
    }

    return Container(
      height: 26,
      // **Load-bearing.** The scroll view below shrink-wraps to the width of
      // its Row, so without this the bar is only as wide as its nine headings
      // — and the Column above it, centring by default, puts that stub in the
      // middle of the window with the backdrop showing either side. The bar is
      // chrome: it spans the window, one colour, headings from the left edge.
      width: double.infinity,
      // The same hairline the toolbar draws under itself, so the two bars read
      // as two bands of chrome rather than one 52px slab of surface2.
      decoration: BoxDecoration(
        color: t.surface2,
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      // Nine menu names do not fit a narrow window, and a menu you cannot
      // reach is worse than one you have to scroll to — so the bar scrolls
      // sideways rather than clipping its last headings away.
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          children: [
            const SizedBox(width: 4),
            for (final menu in menus)
              _MenuButton(title: menu.title, items: menu.items),
            // Nothing to look at: it is here so `Ctrl+Shift+P` opens the same
            // palette this bar builds, rather than the shell building a second
            // one from a list that would drift out of step with these menus.
            _RequestHotkey(
              requests: context.read<LumitUiState>().paletteRequest,
              onRequested: () => _palette(context),
            ),
            // The same, for Ctrl+Space (K-324): the console's effects and comps
            // come from this file for the same reason the palette's commands do.
            _RequestHotkey(
              requests: context.read<LumitUiState>().consoleRequest,
              onRequested: () => _console(context),
            ),
          ],
        ),
      ),
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
          label: l10n.menuNew,
          category: l10n.menuFile,
          run: app.newProject,
        ),
        if (project != null) ...[
          PaletteCommand(
            label: l10n.menuSave,
            category: l10n.menuFile,
            run: () => saveProjectFrb(app, ui, picker: savePicker),
          ),
          PaletteCommand(
            label: l10n.menuSaveAs,
            category: l10n.menuFile,
            run: () =>
                saveProjectFrb(app, ui, forcePicker: true, picker: savePicker),
          ),
          PaletteCommand(
            label: l10n.menuImportFootage,
            category: l10n.menuFile,
            run: () => importFootageFrb(app, picker: footagePicker),
          ),
          PaletteCommand(
            label: l10n.newComposition,
            category: l10n.menuComposition,
            run: () => newCompositionFrb(context, app),
          ),
          PaletteCommand(
            label: l10n.menuUndo,
            category: l10n.menuEdit,
            shortcut: 'Ctrl+Z',
            run: () => undoFrb(app),
          ),
          PaletteCommand(
            label: l10n.menuRedo,
            category: l10n.menuEdit,
            shortcut: 'Ctrl+Shift+Z',
            run: () => redoFrb(app),
          ),
          PaletteCommand(
            label: l10n.menuExport,
            category: l10n.menuFile,
            run: () => exportFrb(context),
          ),
          // Every comp, by name: Enter fronts it in the Viewer and Timeline.
          for (final (comp, name) in app.comps())
            PaletteCommand(
              label: name,
              category: l10n.paletteComps,
              run: () => ui.setSelectedComp(comp),
            ),
          // Every built-in effect: Enter applies it to the selected layer;
          // with none selected it does nothing, exactly like the browser.
          for (final effect in listEffects())
            PaletteCommand(
              label: engineLabel(effect.label),
              category: l10n.menuEffect,
              run: () => ui.selectedLayer.value?.addEffect(name: effect.name),
            ),
        ],
        // Every panel: Enter focuses it in the dock.
        for (final panel in Panel.values)
          PaletteCommand(
            label: panel.title,
            category: l10n.palettePanels,
            run: () => ui.activePanel.value = panel,
          ),
        // The View menu's magnification and preview resolution, so the palette
        // carries them too rather than the menu being the only route.
        for (final zoom in ViewerZoomCommand.values)
          PaletteCommand(
            label: zoom.title,
            category: l10n.menuView,
            run: () => ui.requestViewerZoom(zoom),
          ),
        // Under the Resolution badge rather than View's, because "Full" on its
        // own says nothing about what it is full of.
        for (final resolution in PreviewResolution.values)
          PaletteCommand(
            label: resolution.title,
            category: l10n.menuResolution,
            run: () => ui.setPreviewResolution(resolution),
          ),
        PaletteCommand(
          label: l10n.menuSettings,
          category: l10n.menuEdit,
          run: () => showSettingsWindowFrb(context),
        ),
        if (app.project case final project?)
          PaletteCommand(
            label: l10n.menuProjectSettings,
            category: l10n.menuFile,
            run: () => showProjectSettingsFrb(context, project),
          ),
      ],
    );
  }

  /// The Ctrl+Space console (K-324). Its two halves are built here beside the
  /// menus for the same reason the palette's list is: the effects it applies
  /// and the comps it fronts must be the ones the menus mean.
  Future<void> _console(BuildContext context) async {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // With the graph focused, the console is the graph's own add surface
    // (K-673): the panel opens the same popover wearing the canvas's list —
    // a chosen box lands on the graph — and this one, which applies to the
    // selected layers, stands down.
    if (ui.consoleClaim?.call() ?? false) return;
    final comp = ui.selectedComp;

    void applyEffect(String name) {
      final layers = ui.selectedLayers.value;
      if (layers.isEmpty) return;
      // Every selected layer, as the Effect menu does (K-217).
      for (final target in layers) {
        target.addEffect(name: name);
      }
      app.notifyDocumentChanged();
    }

    // A saved preset's whole stack, to every selected layer — the Effects &
    // presets panel's own rules (K-523): read once, applied per layer, each
    // layer's refusal leaving the rest of the batch standing.
    void applyPreset(BridgePresetInfo preset) {
      final layers = ui.selectedLayers.value;
      if (layers.isEmpty) return;
      final String text;
      try {
        text = File(preset.path).readAsStringSync();
      } catch (_) {
        return;
      }
      for (final layer in layers) {
        try {
          layer.loadPreset(text: text);
        } catch (_) {}
      }
      app.notifyDocumentChanged();
    }

    await showFxConsoleFrb(
      context: context,
      // The popover opens on the mouse (K-325): the shell records where the
      // pointer last was, because the key event itself has no position.
      anchor: lastKnownPointerPosition,
      model: FxConsoleModel(
        keyHint: l10n.fxConsoleKey,
        footer: l10n.fxConsoleApplies,
        onSnapshot: comp == null ? null : () => saveSnapshotFrb(app, ui),
        entries: [
          // Effects first — the overwhelmingly common reason to open this.
          for (final effect in listEffects())
            FxConsoleEntry(
              label: engineLabel(effect.label),
              kind: FxConsoleKind.effect,
              group: engineLabel(effect.categoryLabel),
              run: () => applyEffect(effect.name),
            ),
          // The saved presets beside them, under the Presets kicker the
          // board's category strip draws.
          for (final preset in listPresets())
            FxConsoleEntry(
              label: preset.name,
              kind: FxConsoleKind.effect,
              group: l10n.fxConsolePresets,
              run: () => applyPreset(preset),
            ),
          // Then the comps.
          for (final (each, name) in app.comps())
            FxConsoleEntry(
              label: name,
              kind: FxConsoleKind.composition,
              run: () => ui.setSelectedComp(each),
            ),
        ],
      ),
    );
  }
}

// --- The tree -------------------------------------------------------------

/// Every menu, in bar order. Pure construction: it reads the document and the
/// shell state and hands back rows, so the two renderers below (and a test)
/// all see exactly the same bar.
List<MenuSection> lumitMenus(
  BuildContext context,
  LumitState app, {
  Future<String?> Function()? openPicker,
  Future<String?> Function()? savePicker,
  Future<List<String>> Function()? footagePicker,
  Future<String?> Function()? aeProjectPicker,
  VoidCallback? palette,
}) {
  final ui = context.read<LumitUiState>();
  final project = app.project;
  // Null while no project is loaded, so every document item is disabled rather
  // than throwing when pressed. A *dead* reference gets the same answer:
  // openProject clears the engine's registry before _adopt lands the new
  // reference, so a rebuild inside that window holds a project every call
  // refuses. One build with the rows disabled; the adopt notification
  // rebuilds with the live one.
  BridgeHistory? history;
  String? projectPath;
  try {
    history = project?.history();
    projectPath = project?.path();
  } catch (_) {}
  final comp = ui.selectedComp;
  final layer = ui.selectedLayer.value;
  final layers = ui.selectedLayers.value;

  /// Wrap a composition action: null (so the item greys out) when no
  /// composition is fronted, else the action followed by a redraw.
  VoidCallback? onComp(void Function(CompositionReference) run) {
    if (comp == null) return null;
    return () {
      run(comp);
      app.notifyDocumentChanged();
    };
  }

  /// The same, for a command that acts on the selected *layer* — greyed out
  /// with nothing selected rather than offered and inert.
  VoidCallback? onLayer(void Function(LayerReference) run) {
    if (layer == null) return null;
    return () => run(layer);
  }

  return [
    (
      title: l10n.menuFile,
      items: () => [
        MenuEntry(l10n.menuNew, app.newProject, action: 'file.new'),
        MenuEntry(
            l10n.menuOpenProject, () => openProjectFrb(app, picker: openPicker),
            action: 'file.open'),
        MenuEntry.submenu(l10n.menuOpenRecent, [
          if (ui.workspace.recentProjects.isEmpty)
            MenuEntry(l10n.menuNothingYet, null)
          else
            for (final path in ui.workspace.recentProjects)
              MenuEntry(path, () => app.openProject(path)),
        ]),
        MenuEntry.divider(),
        // Closing a project puts an empty one in its place, because the shell
        // always has a document: the engine's `close` is what `newProject`
        // already does to the one it replaces, so this is the same road the
        // application takes at launch, walked deliberately.
        MenuEntry(l10n.menuCloseProject,
            project == null ? null : app.newProject),
        // Save is only meaningful once there is a project; without a path it
        // behaves as Save as, which is what the engine's empty-path refusal
        // makes us handle explicitly.
        MenuEntry(
            l10n.menuSave,
            project == null
                ? null
                : () => saveProjectFrb(app, ui, picker: savePicker),
            action: 'file.save'),
        MenuEntry(
            l10n.menuSaveAs,
            project == null
                ? null
                : () => saveProjectFrb(app, ui,
                    forcePicker: true, picker: savePicker),
            action: 'file.save.as'),
        MenuEntry.divider(),
        // Import footage stands in the menu proper (owner, 2026-08-25,
        // superseding the one-Import-home grouping of 2026-08-21): it is the
        // everyday act, and a submenu hop for it earned its promotion.
        MenuEntry(
            l10n.menuImportFootage,
            project == null
                ? null
                : () => importFootageFrb(app, picker: footagePicker),
            action: 'file.import'),
        // The After Effects route keeps the submenu. The Bridge-bundle entry
        // is gone (owner, repeatedly): the .aep front door (K-418) is the
        // import; the bundle format itself remains supported for anything
        // that already produced one, just not offered here.
        MenuEntry.submenu(l10n.menuImport, [
          // Not gated on a project: an import *replaces* whatever is loaded,
          // the way opening a `.lum` does, so it is offered with none.
          MenuEntry(
              l10n.menuImportAe,
              () => importAeBundleFrb(context, app,
                  picker: aeProjectPicker ?? pickAeProject)),
        ]),
        MenuEntry(
            l10n.menuExport, comp == null ? null : () => exportFrb(context),
            action: 'file.export'),
        MenuEntry.divider(),
        // The project's own settings, kept apart from Settings because Settings
        // is this machine's and these travel in the `.lum` (K-286).
        MenuEntry(
            l10n.menuProjectSettings,
            project == null
                ? null
                : () => showProjectSettingsFrb(context, project),
            action: 'project.settings'),
        MenuEntry.divider(),
        // Not in the specified list, and kept: recovering work beside a project
        // is the one command whose absence costs a day's work.
        MenuEntry(l10n.menuRecover,
            projectPath == null ? null : () => _recover(context, app)),
      ]
    ),
    (
      title: l10n.menuEdit,
      items: () => [
        MenuEntry(l10n.menuUndo,
            (history?.canUndo ?? false) ? () => undoFrb(app) : null,
            action: 'edit.undo'),
        MenuEntry(l10n.menuRedo,
            (history?.canRedo ?? false) ? () => redoFrb(app) : null,
            action: 'edit.redo'),
        // The journal as a list you can read and click (K-688). Undo and redo
        // above it walk the same list one step at a time.
        MenuEntry(l10n.menuHistory,
            project == null ? null : () => showHistoryFrb(context, app)),
        MenuEntry.divider(),
        // Copy takes the finest thing that is selected (K-300): the keyframes a
        // panel has claimed, else the picked effects, else the selected layer
        // whole — transform, keyframes, masks, paint, effects and switches — as
        // the document text the engine hands back (K-275). Cut is that plus the
        // delete, so the two can never disagree about what "the selection" was.
        MenuEntry(l10n.menuCut,
            _somethingSelected(ui) ? () => cutSelectionFrb(app, ui) : null,
            action: 'edit.cut'),
        MenuEntry(l10n.menuCopy,
            _somethingSelected(ui) ? () => copySelectionFrb(ui) : null,
            action: 'edit.copy'),
        // Paste puts a layer at the playhead — or at the time it was copied
        // from, for the person rebuilding a moment in a second comp (Settings →
        // Interface). An effect always lands with its first keyframe at the
        // playhead, whichever way that setting is: what is being placed is an
        // animation rather than a position.
        MenuEntry(l10n.menuPaste, _pasteAction(app, ui, comp, layer),
            action: 'edit.paste'),
        MenuEntry(
            l10n.delete,
            layers.isEmpty
                ? null
                : () {
                    for (final l in layers) {
                      l.delete();
                    }
                    ui.clearSelection();
                    app.notifyDocumentChanged();
                  },
            action: 'edit.delete.selection'),
        MenuEntry.divider(),
        MenuEntry(l10n.menuDuplicate, onLayer((l) {
          l.duplicate();
          app.notifyDocumentChanged();
        }), action: 'layer.duplicate'),
        MenuEntry(l10n.menuSplitLayer, onComp((c) => _splitAtPlayhead(ui)),
            action: 'layer.split'),
        MenuEntry(l10n.menuSelectAll,
            comp == null ? null : () => ui.setSelection(comp.getLayers()),
            action: 'edit.select.all'),
        MenuEntry(l10n.menuDeselectAll, ui.clearSelection,
            action: 'edit.deselect.all'),
        MenuEntry.divider(),
        // Windows and Linux keep Preferences under Edit, which is where every
        // application those users know puts it. macOS moves this same row into
        // the application menu (see [platformMenusFor]), which is where every
        // application *those* users know puts it.
        MenuEntry(l10n.menuSettings, () => showSettingsWindowFrb(context),
            action: 'app.settings'),
      ]
    ),
    (
      title: l10n.menuComposition,
      items: () => [
        MenuEntry(l10n.newComposition,
            project == null ? null : () => newCompositionFrb(context, app),
            action: 'comp.new'),
        MenuEntry.divider(),
        MenuEntry(l10n.compositionSettingsEllipsis,
            comp == null ? null : () => _compSettings(context, app),
            action: 'comp.settings'),
        // Make the comp be the stretch you marked (K-686), and the frame be
        // the rectangle you swept (K-687). Each is greyed until there is one:
        // a comp with no work area is already its own work area (K-203), and
        // with no region there is no rectangle to crop to.
        MenuEntry(l10n.menuTrimCompToWorkArea, _trimAction(app, comp)),
        MenuEntry(
            l10n.menuCropCompToRegion,
            comp == null || ui.regionOfInterest == null
                ? null
                : () {
                    comp.cropToRegion(region: ui.regionOfInterest!);
                    // The comp *is* the region now, so the region has nothing
                    // left to say; leaving it set would window the new frame
                    // down to a corner of what was just cropped.
                    ui.setRegionOfInterest(null);
                    app.notifyDocumentChanged();
                  }),
        MenuEntry.divider(),
        // "Export", never "render", for anything the user sees (glossary §9).
        // Adding to the queue is what the export dialog does, so this opens
        // it rather than queueing something nobody has said where to write.
        MenuEntry(l10n.menuAddToExportQueue,
            comp == null ? null : () => exportFrb(context),
            action: 'export.queue.add'),
        MenuEntry.divider(),
        // Comp-level markers, including the beat pass, which makes them
        // (docs/09 §10) — the layer's own markers are Layer ▸ Markers.
        MenuEntry(l10n.menuAddMarkerAtPlayhead,
            onComp((c) => _markerAtPlayhead(ui, c)),
            action: 'marker.add'),
        // Beat detection reads the whole comp's audio and can take seconds, so
        // it runs off-thread; a comp with nothing sounding in it refuses, and
        // says so on the status line rather than by leaving the Timeline
        // exactly as it was.
        MenuEntry(
            l10n.menuDetectBeats,
            onComp((c) => c
                .detectBeats(options: BridgeBeatOptions.standard())
                .then((found) {
              if (found.placed == 0) app.postNotice(l10n.beatsNoneFound);
            }, onError: (_) => app.postNotice(l10n.beatsNoSound)))),
        MenuEntry(
            l10n.menuClearBeatMarkers, onComp((c) => c.clearBeatMarkers())),
      ]
    ),
    (
      title: l10n.menuLayer,
      items: () => [
        MenuEntry.submenu(l10n.menuNew, [
          MenuEntry(l10n.menuSolid, onComp((c) => c.addSolidLayer())),
          MenuEntry(l10n.menuText, onComp((c) => c.addTextLayer())),
          MenuEntry(l10n.menuCamera, onComp((c) => c.addCameraLayer())),
          // The three light kinds are their own rows rather than one row and
          // a dropdown: which kind you want is known before you make it, and
          // an area light is a different thing to reach for than a point.
          MenuEntry(
              l10n.menuPointLight, onComp((c) => c.addLightLayer(kind: 0))),
          MenuEntry(
              l10n.menuSpotLight, onComp((c) => c.addLightLayer(kind: 1))),
          MenuEntry(
              l10n.menuAreaLight, onComp((c) => c.addLightLayer(kind: 2))),
          MenuEntry(l10n.menuAdjustment, onComp((c) => c.addAdjustmentLayer())),
          MenuEntry(l10n.menuNull, onComp((c) => c.addNullLayer())),
          MenuEntry(l10n.menuSequence, onComp((c) => c.addSequenceLayer())),
        ]),
        // What the layer *is*, as opposed to what it is doing: its name, and a
        // Solid's own size and colour (K-444's dialogue pattern).
        MenuEntry(
            l10n.menuLayerSettings,
            onLayer((l) async {
              if (await showLayerSettingsFrb(context: context, layer: l)) {
                app.notifyDocumentChanged();
              }
            })),
        MenuEntry.divider(),
        MenuEntry.submenu(l10n.menuMask, maskRows(context, app, ui)),
        MenuEntry.submenu(
            l10n.menuMaskAndShapePath, maskAndShapePathRows(app, ui)),
        MenuEntry.submenu(l10n.menuTransform, transformRows(app, ui)),
        // Audio ▸ — what to do with the *sound* of a layer that has some.
        // Detach puts it on a row of its own, muting the picture's row, so it
        // can be cut and ridden in the audio surfaces without the picture
        // coming along (K-701). Greyed on a layer that is already nothing but
        // sound; a layer that turns out to make none refuses when pressed and
        // says so in the status line, because the answer costs a probe of the
        // media and a menu cannot wait for one.
        MenuEntry.submenu(l10n.menuAudio, [
          MenuEntry(
            l10n.menuDetachAudio,
            _detachable(layer)
                ? onLayer((l) async {
                    try {
                      await l.detachAudio();
                      app.notifyDocumentChanged();
                    } catch (_) {
                      app.postNotice(l10n.detachAudioNoSound);
                    }
                  })
                : null,
          ),
        ]),
        // The selected layer's Retime (K-197). In the menu as well as on the
        // keyboard (K-198's lesson: a command whose only route is a chord has no
        // route the day something intercepts the chord). The command names what
        // it will do, so a layer that already has one offers to take it away.
        // Greyed out on a Sequence layer: its clips carry the retiming and are
        // ramped in the sequence view (K-075), so there is nothing here for the
        // command to switch on. Said with a disabled row rather than an error
        // after the click.
        MenuEntry(_retimeLabel(layer),
            _retimeable(layer) ? onLayer((l) => app.toggleRetime(l)) : null,
            action: 'layer.retime.enable'),
        // In and out of the clip-editing surface, for anyone — the Vegas
        // preference decides what an *import* becomes (K-246), never what a
        // layer is allowed to be. Offered here and on a layer's right-click.
        // Coming back out is offered whenever going in is, because a user who
        // tries it has to be able to change their mind.
        MenuEntry(
            _sequenced(layer)
                ? l10n.menuConvertToFootageLayer
                : l10n.menuConvertToSequenceLayer,
            _convertible(layer)
                ? onLayer((l) {
                    try {
                      if (_sequenced(layer)) {
                        l.convertFromSequenced();
                      } else {
                        l.convertToSequenced();
                      }
                      app.notifyDocumentChanged();
                    } catch (_) {
                      // A row of several clips refuses, and says so through the
                      // status line rather than taking the interface down.
                    }
                  })
                : null,
            action: 'layer.sequence.convert'),
        flowRow(app, ui),
        threeDRow(app, ui),
        MenuEntry.submenu(l10n.menuMarkers, markerRows(app, ui)),
        MenuEntry.divider(),
        MenuEntry.todo(l10n.menuPreserveTransparency),
        MenuEntry.submenu(l10n.menuBlendingMode, blendRows(app, ui)),
        blendStepRow(app, ui, by: 1),
        blendStepRow(app, ui, by: -1),
        MenuEntry.submenu(l10n.menuTrackMatte, matteRows(app, ui)),
        MenuEntry.submenu(l10n.menuLayerStyles, styleRows(app, ui)),
        MenuEntry.divider(),
        MenuEntry.todo(l10n.menuReveal),
        // Create ▸ — what a layer can be turned *into*, keeping the layer it
        // was (K-608). Both rows are live only on a Type layer, said by greying
        // out rather than by an error after the click; the copy lands directly
        // above the original, where a duplicate goes.
        MenuEntry.submenu(l10n.menuCreate, [
          MenuEntry(
              l10n.menuCreateShapesFromText,
              _typed(layer)
                  ? onLayer((l) {
                      try {
                        l.createShapesFromText(
                            frame: ui.playheadFrame.value);
                        app.notifyDocumentChanged();
                      } catch (_) {
                        // A line with no ink has no art to make, and says so
                        // through the status line rather than taking the
                        // interface down.
                      }
                    })
                  : null),
          MenuEntry(
              l10n.menuCreatePointsFromText,
              _typed(layer)
                  ? onLayer((l) {
                      try {
                        l.createPointsFromText();
                        app.notifyDocumentChanged();
                      } catch (_) {}
                    })
                  : null),
        ]),
        MenuEntry.divider(),
        MenuEntry.todo(l10n.menuCamera),
        MenuEntry.todo(l10n.menuAutoOutline),
        // Pre-compose… is live only with a comp open and something selected in
        // it — the menu says so by greying out rather than by failing.
        MenuEntry(
            l10n.menuPreCompose,
            comp == null || layers.isEmpty
                ? null
                : () => showPrecomposeDialogFrb(
                      context: context,
                      comp: comp,
                      selectedLayers: layers,
                      ui: ui,
                      workspace: ui.workspace,
                    ),
            action: 'layer.precompose'),
        // The light fold beside the heavy one (K-702). Group is live with
        // something selected; Ungroup only while the selection is actually in
        // a group, so the row greys out rather than doing nothing when
        // pressed. Both go through the shell's own command, which is the one
        // implementation the keyboard reaches too.
        MenuEntry.divider(),
        MenuEntry(
            l10n.menuGroupLayers,
            comp == null || layers.isEmpty
                ? null
                : () {
                    groupSelectedLayers(
                      comp: comp,
                      layerIds: [for (final l in layers) l.internallayerId],
                      name: l10n.groupDefaultName,
                    );
                    app.notifyDocumentChanged();
                  },
            action: 'layer.group'),
        MenuEntry(
            l10n.menuUngroup,
            comp == null ||
                    !ui.model.groups
                        .any((g) => g.members.any(ui.selectedLayerIds.contains))
                ? null
                : () {
                    ungroupSelection(
                      comp: comp,
                      groups: ui.model.groups,
                      layerIds: ui.selectedLayerIds,
                    );
                    app.notifyDocumentChanged();
                  },
            action: 'layer.ungroup'),
      ]
    ),
    (title: l10n.menuEffect, items: () => _effectMenu(app, layers)),
    (
      title: l10n.menuAnimation,
      items: () => [
        MenuEntry.todo(l10n.menuSaveAnimationPreset),
        MenuEntry.todo(l10n.menuApplyAnimationPreset),
        MenuEntry.divider(),
        // The four keyframe commands act on the property rows the Timeline has
        // picked, and on the keys those rows carry **under the playhead**: the
        // key selection itself is the panel's and is never published, so a
        // menu claiming to act on it would be guessing.
        setKeyframeRow(app, ui),
        toggleHoldRow(app, ui),
        keyframeInterpolationRow(context, app, ui),
        keyframeSpeedRow(context, app, ui),
        MenuEntry.divider(),
        animateTextRow(app, ui),
        // A K-609 animator carries exactly one range selector and it arrives
        // with the animator, so there is nothing for this to add until more
        // than one selector exists to have.
        MenuEntry.todo(l10n.menuAddTextSelector),
        MenuEntry.divider(),
        addExpressionRow(context, app, ui),
        MenuEntry.submenu(l10n.menuSeparateDimensions, axisModeRows(app, ui)),
        trackCameraRow(app, ui),
        MenuEntry.todo(l10n.menuTrackMotion),
        MenuEntry.divider(),
        // The Reveal family (K-684): `U`'s own machinery under the menu's
        // words, each row a wider rule than the one above it. They act on the
        // selection, or on the whole composition when nothing is selected —
        // which is why they are live whenever a comp is open, and greyed
        // rather than absent when none is.
        for (final reveal in const [
          (RevealFilter.keyframed, 'reveal.animated'),
          (RevealFilter.animated, null),
          (RevealFilter.modified, null),
        ])
          MenuEntry(
            switch (reveal.$1) {
              RevealFilter.keyframed => l10n.menuRevealPropertiesWithKeyframes,
              RevealFilter.animated => l10n.menuRevealPropertiesWithAnimation,
              RevealFilter.modified => l10n.menuRevealAllModifiedProperties,
            },
            comp == null ? null : () => ui.requestRevealFilter(reveal.$1),
            action: reveal.$2,
          ),
      ]
    ),
    (
      title: l10n.menuView,
      items: () => [
        // Magnification: the same three jumps the Viewer's own keyboard makes
        // (docs/07 §2.2). Greyed with no composition fronted, because there is
        // no picture in the panel to magnify.
        for (final zoom in ViewerZoomCommand.values)
          MenuEntry(
            zoom.title,
            comp == null ? null : () => ui.requestViewerZoom(zoom),
            action: zoom.action,
          ),
        MenuEntry.divider(),
        // Preview resolution (§2.2 item 2): how many pixels the engine is
        // asked for, ticked so the menu says which one is in force.
        MenuEntry.submenu(l10n.menuResolution, [
          for (final resolution in PreviewResolution.values)
            MenuEntry.option(
              resolution.title,
              () => ui.setPreviewResolution(resolution),
              // Only three of the five have a chord of their own (§15);
              // Auto and Third are menu and bar only.
              action: resolution.action,
              checked: () => ui.previewResolution == resolution,
            ),
        ]),
        MenuEntry.divider(),
        // The marks over the picture (docs/07 §2.2 items 5–6, K-683). All
        // three are the Viewer's own view menu under other names, so they are
        // toggles: turning the rulers on to drag a guide out and ticking Snap
        // to grid is two ticks, not two trips to the View menu (K-520).
        //
        // Show grid carries no chord: `Ctrl+'` belongs to the *transparency*
        // grid (§15's table), which is a different grid and a different
        // question, and a row advertising a key that does something else is
        // worse than a row with no key at all.
        MenuEntry.toggle(
          l10n.menuShowGrid,
          () => ui.setViewerOverlays(grid: !ui.viewerOverlays.grid),
          checked: () => ui.viewerOverlays.grid,
        ),
        MenuEntry.toggle(
          l10n.menuShowRuler,
          () => ui.setViewerOverlays(rulers: !ui.viewerOverlays.rulers),
          action: 'viewer.rulers.toggle',
          checked: () => ui.viewerOverlays.rulers,
        ),
        // The layer-controls switch (K-217, K-466): the wireframes, the
        // handles and the hover highlight, on and off together. The bar's own
        // view menu carries the same switch under its own name — they are one
        // switch until the full wireframe display mode of §2.2 item 5 gives
        // this row something of its own to turn on.
        MenuEntry(l10n.menuShowWireframe,
            () => ui.setViewerLayerControls(!ui.viewerLayerControls),
            checked: ui.viewerLayerControls),
        // Whether the grid's own lines are things a dragged layer lands on,
        // over and above the guides (K-683). The magnet on the toolbar is what
        // decides whether *any* of it engages; this says what is in the list.
        MenuEntry.toggle(
          l10n.menuSnapToGrid,
          () => ui.tools.snapToGrid = !ui.tools.snapToGrid,
          checked: () => ui.tools.snapToGrid,
        ),
      ]
    ),
    (
      title: l10n.menuWindow,
      items: () => [
        MenuEntry.submenu(l10n.menuWorkspace, [
          for (final preset in WorkspacePreset.values)
            MenuEntry(
                preset.title, () => ui.workspace.applyWorkspacePreset(preset),
                checked: ui.workspace.activePreset == preset),
          // The user's own, under the presets, in the strip's own order.
          if (ui.workspace.userWorkspaces.isNotEmpty) MenuEntry.divider(),
          for (final saved in ui.workspace.userWorkspaces)
            MenuEntry(
                saved.name, () => ui.workspace.applyUserWorkspace(saved.name),
                checked: ui.workspace.activeUserWorkspace == saved.name),
          MenuEntry.divider(),
          ..._userWorkspaceRows(context, app, ui),
          MenuEntry.divider(),
          MenuEntry(l10n.menuResetWorkspace, ui.resetLayout),
        ]),
        // A chord for the arrangement on screen (K-574): the engine's nine
        // `workspace.switch.N` actions count *slots* on the strip, so what
        // is bound is the position rather than the name — which is what the
        // dialogue says, and what makes the key reach the same place next
        // launch.
        MenuEntry(l10n.menuAssignWorkspaceShortcut,
            assignWorkspaceShortcutAction(context, app, ui)),
        MenuEntry.divider(),
        // Every panel, ticked when it is in the arrangement. Toggling one adds
        // or drops its pane and persists the layout, so a panel you closed stays
        // closed across a restart — and the menu stays open while you do it
        // (K-520), because turning three panels on is three ticks, not three
        // trips to the Window menu.
        for (final panel in Panel.values)
          MenuEntry.toggle(
            panel.title,
            () {
              setPanelVisible(ui.split, panel, !panelVisible(ui.split, panel));
              ui.workspace.touch();
            },
            checked: () => panelVisible(ui.split, panel),
          ),
        MenuEntry.divider(),
        MenuEntry(
            l10n.menuExportQueue, () => showExportQueueFrb(context: context)),
        MenuEntry(l10n.menuCommandPalette, palette, action: 'palette.open'),
      ]
    ),
    (
      title: l10n.menuHelp,
      items: () => [
        MenuEntry(l10n.menuAboutLumit, () => showAboutWindowFrb(context)),
        MenuEntry.live(
          ui.updates,
          () => updateMenuEntry(context, app, ui, savePicker: savePicker),
        ),
        MenuEntry.divider(),
        // The documentation, in whatever the user reads the web with (K-279).
        // Both are pages on docs.lumitlab.com rather than one of them being
        // the marketing site: "online guides" is where you are taught, and
        // that is the walkthrough, not the download page.
        MenuEntry(l10n.menuLumitHelp, () => _openLink(app, lumitDocsUrl)),
        MenuEntry(
            l10n.menuLumitOnlineGuides, () => _openLink(app, lumitGuidesUrl)),
        MenuEntry.divider(),
        MenuEntry.toggle(
          l10n.menuEnableDebugPanel,
          () {
            setPanelVisible(
                ui.split, Panel.debug, !panelVisible(ui.split, Panel.debug));
            ui.workspace.touch();
          },
          checked: () => panelVisible(ui.split, Panel.debug),
        ),
      ]
    ),
  ];
}

/// The Effect menu: one submenu per effect category (K-090), each applying its
/// effect to every selected layer. Disabled outright with nothing selected —
/// there is nowhere for an effect to go.
List<MenuEntry> _effectMenu(LumitState app, List<LayerReference> layers) => [
      for (final group in _effectGroups().entries)
        MenuEntry.submenu(engineLabel(group.key), [
          for (final effect in group.value)
            MenuEntry(
              engineLabel(effect.label),
              layers.isEmpty
                  ? null
                  : () {
                      for (final layer in layers) {
                        layer.addEffect(name: effect.name);
                      }
                      app.notifyDocumentChanged();
                    },
            ),
        ]),
    ];

/// Every built-in effect, grouped by its heading, in the engine's own order.
///
/// Read once: the catalogue is fixed for the run, and the menu bar rebuilds on
/// every document change — a bridge call per rebuild is exactly the cost the
/// hover-hot paths are budgeted against.
Map<String, List<BridgeEffectInfo>> _effectGroups() =>
    _effectGroupsCache ??= () {
      final groups = <String, List<BridgeEffectInfo>>{};
      for (final effect in listEffects()) {
        groups.putIfAbsent(effect.categoryLabel, () => []).add(effect);
      }
      return groups;
    }();

Map<String, List<BridgeEffectInfo>>? _effectGroupsCache;

/// Whether this layer is a Sequence layer.
bool _sequenced(LayerReference? layer) {
  if (layer == null) return false;
  try {
    return layer.getKind() == BridgeLayerKind.sequence;
  } catch (_) {
    return false;
  }
}

/// Whether this layer can cross between footage and sequence at all. Only
/// footage has clips to cut, and only a sequence has any to put back.
/// Whether this is a Type layer — the one kind Create ▸ has anything to offer
/// (K-608).
bool _typed(LayerReference? layer) {
  if (layer == null) return false;
  try {
    return layer.getKind() == BridgeLayerKind.text;
  } catch (_) {
    return false;
  }
}

bool _convertible(LayerReference? layer) {
  if (layer == null) return false;
  try {
    final kind = layer.getKind();
    return kind == BridgeLayerKind.footage || kind == BridgeLayerKind.sequence;
  } catch (_) {
    return false;
  }
}

/// Whether this layer has a sound that could be put on a row of its own
/// (K-701). A layer that is *already* only sound has nothing to separate it
/// from; whether the rest of them actually make any is the engine's answer,
/// and it costs a probe, so it is given as a refusal rather than a grey row.
bool _detachable(LayerReference? layer) {
  if (layer == null) return false;
  try {
    return layer.getKind() != BridgeLayerKind.audio;
  } catch (_) {
    return false;
  }
}

/// Whether this layer can carry a Retime at all. A Sequence layer cannot: its
/// clips each have one of their own (K-075).
bool _retimeable(LayerReference? layer) {
  if (layer == null) return false;
  try {
    return layer.getKind() != BridgeLayerKind.sequence;
  } catch (_) {
    return false;
  }
}

/// What the Retime item says.
String _retimeLabel(LayerReference? layer) {
  if (layer == null) return l10n.menuEnableRetime;
  try {
    if (layer.getKind() == BridgeLayerKind.sequence) {
      return l10n.menuRetimeSequence;
    }
    return layer.getRetimeProperty() == null
        ? l10n.menuEnableRetime
        : l10n.menuDisableRetime;
  } catch (_) {
    return l10n.menuEnableRetime;
  }
}

// --- The commands ---------------------------------------------------------
//
// Free functions, not methods, because the keyboard runs the same commands the
// menu does (K-199 dispatch lives in main.dart) and a shortcut that took a
// different path than its menu item would be two implementations to keep
// honest. [saveProjectFrb] was the first of these (K-203); the rest followed
// when the menu grew shortcuts.

/// Follow a Help-menu link, and say so in the status line when the desktop
/// would not take it — a machine with no browser registered leaves a menu row
/// that does nothing at all, which reads as broken rather than as a machine
/// without a browser.
Future<void> _openLink(LumitState app, String url) async {
  if (await openExternalLink(url)) return;
  app.postNotice(l10n.couldNotOpenLink(url), error: true);
}

Future<void> openProjectFrb(LumitState app,
    {Future<String?> Function()? picker}) async {
  final path = await (picker ?? pickProjectToOpen)();
  if (path == null) return;
  await app.openProject(path);
}

Future<void> importFootageFrb(LumitState app,
        {Future<List<String>> Function()? picker}) async =>
    app.importFootagePaths(await (picker ?? pickFootage)());

/// Import an After Effects project, then show its report (docs/11 §9).
///
/// One function for both File ▸ Import rows: the `.aep` picker and the Bridge
/// bundle folder picker hand it a path, and which route the path takes is the
/// engine's decision from the bytes (K-418).
///
/// The report is shown whatever it says — an import that adjusted nothing still
/// opens the window, because "everything came across untouched" is the answer
/// the user came for. Something that is neither posts a notice instead and
/// leaves the open project alone.
Future<void> importAeBundleFrb(BuildContext context, LumitState app,
    {Future<String?> Function()? picker}) async {
  final path = await (picker ?? pickAeProject)();
  if (path == null) return;
  final report = await app.importAeBundle(path);
  if (report == null || !context.mounted) return;
  await showAeImportReport(context: context, report: report);
}

/// Make a composition and front it — a comp you just made is the one you want
/// to work on.
Future<void> newCompositionFrb(BuildContext context, LumitState app) async {
  final comp = await app.newComposition(context);
  if (comp != null && context.mounted) {
    context.read<LumitUiState>().setSelectedComp(comp);
  }
}

Future<void> exportFrb(BuildContext context) async {
  final comp = context.read<LumitUiState>().selectedComp;
  if (comp == null) return;
  await showExportDialogFrb(context: context, comp: comp);
}

void undoFrb(LumitState app) {
  app.project?.undo();
  app.notifyDocumentChanged();
}

void redoFrb(LumitState app) {
  app.project?.redo();
  app.notifyDocumentChanged();
}

/// Whether Cut and Copy have anything to act on — an effect picked out of a
/// stack, or a layer. Keyframes are not counted: the panel holding them claims
/// the chord, and a menu row that ungreyed on a selection the menu cannot see
/// would be guessing.
bool _somethingSelected(LumitUiState ui) =>
    ui.selectedEffects.value.isNotEmpty || ui.selectedLayer.value != null;

/// What Copy takes, finest selection first (K-300): the keyframes a panel has
/// claimed, else the picked effects, else the selected layer. Returns whether
/// anything was copied, so the keyboard can leave the chord unhandled when
/// there was nothing to take.
///
/// One function for the Edit menu and the `Mod+C` chord, because a menu row and
/// a shortcut that disagreed about what "the selection" is would be the bug
/// this fixes, one layer down.
bool copySelectionFrb(LumitUiState ui) {
  if (ui.copyClaim?.call() ?? false) return true;
  if (ui.selectedEffectsLayer case final layer?
      when ui.selectedEffects.value.isNotEmpty) {
    try {
      ui.copyEffectsToClipboard(
          layer.copyEffects(effects: ui.selectedEffects.value));
      return true;
    } catch (_) {
      // The effects went away under the selection; the clipboard keeps what it
      // had, and the layer below is not a silent substitute for what was asked.
      return false;
    }
  }
  final layer = ui.selectedLayer.value;
  if (layer == null) return false;
  ui.copyLayerToClipboard(layer.copyLayer());
  return true;
}

/// Cut is Copy plus the removal, so the two can never disagree about what the
/// selection was (K-275, extended to effects by K-300).
bool cutSelectionFrb(LumitState app, LumitUiState ui) {
  final effects = ui.selectedEffects.value;
  final effectsLayer = ui.selectedEffectsLayer;
  final layer = ui.selectedLayer.value;
  if (!copySelectionFrb(ui)) return false;
  if (effectsLayer != null && effects.isNotEmpty) {
    for (final instance in effectsLayer.getEffects()) {
      if (effects.contains(instance.getInfo().id)) {
        effectsLayer.removeEffect(effect: instance);
      }
    }
    ui.clearEffectSelection();
    app.notifyDocumentChanged();
    return true;
  }
  if (layer == null) return false;
  layer.delete();
  ui.clearSelection();
  app.notifyDocumentChanged();
  return true;
}

/// What Paste does with whatever is on the clipboard (K-275), or `null` when
/// there is nothing to paste — or nowhere to put it.
///
/// A layer goes into the composition on screen: at the playhead by default, or
/// at the time it was copied from when Settings → Interface says so. An effect
/// goes onto the selected layer, always with its first keyframe at the
/// playhead.
/// Paste for the `Mod+V` chord: a panel holding keyframes takes it first
/// (K-300, the same claim Delete has used since K-234), else the clipboard's
/// layer or effects go in. Returns whether anything was pasted.
///
/// The Edit menu's own row stays on [_pasteAction]: keyframes have never been
/// one of its cases, and a row that greys on the clipboard being empty must not
/// ungrey merely because the Timeline is open.
Future<bool> pasteSelectionFrb(
  LumitState app,
  LumitUiState ui,
  CompositionReference? comp,
  LayerReference? layer,
) async {
  if (ui.pasteClaim?.call() ?? false) return true;
  // Nothing in the tray: something may have been copied in another Lumit
  // window, or in this one before something else on the machine overwrote the
  // tray's mirror (K-302).
  if (ui.clipboard.isEmpty) await ui.adoptSystemClipboard();
  final paste = _pasteAction(app, ui, comp, layer);
  if (paste == null) return false;
  paste();
  return true;
}

VoidCallback? _pasteAction(
  LumitState app,
  LumitUiState ui,
  CompositionReference? comp,
  LayerReference? layer,
) {
  final text = ui.clipboard.text;
  if (text == null) return null;
  switch (ui.clipboard.kind) {
    case ClipboardKind.layer:
      if (comp == null) return null;
      return () {
        final pasted = comp.pasteLayer(
          text: text,
          atFrame: ui.workspace.interface.pasteLayersAtOriginalTime
              ? null
              : ui.playheadFrame.value,
        );
        // Selecting what was just pasted is what every editor does, and it is
        // also what makes a second paste land somewhere you can see.
        ui.setSelection([pasted]);
        app.notifyDocumentChanged();
      };
    case ClipboardKind.effects:
      if (layer == null) return null;
      return () {
        layer.pasteEffects(text: text, atFrame: ui.playheadFrame.value);
        app.notifyDocumentChanged();
      };
    case null:
      return null;
  }
}

/// **Trim comp to work area** (K-686), offered only when there is a work area
/// to trim to: a comp nobody has narrowed is already its own work area (K-203),
/// so the row would promise a change it could not make.
///
/// The read is inside the menu's own `items()` closure, so it costs one call on
/// a deliberate press rather than one per rebuild of the bar — and it is
/// guarded, like the bar's other engine reads, so a reference that has just
/// gone dead greys the row instead of taking the menu down with it.
VoidCallback? _trimAction(LumitState app, CompositionReference? comp) {
  if (comp == null) return null;
  try {
    if (comp.getWorkArea() == null) return null;
  } catch (_) {
    return null;
  }
  return () {
    comp.trimToWorkArea();
    app.notifyDocumentChanged();
  };
}

/// Razor the selected layer at the playhead. Only Sequence layers hold clips,
/// so on anything else the engine declines and nothing happens.
void _splitAtPlayhead(LumitUiState ui) {
  final layer = ui.selectedLayer.value;
  if (layer == null) return;
  try {
    layer.cutClipAt(frame: ui.playheadFrame.value);
  } catch (_) {}
}

/// The menu's *Add marker* — the same call the keyboard makes, so the two
/// cannot drift into different ideas of what dropping a marker means (the
/// one-per-frame rule included).
void _markerAtPlayhead(LumitUiState ui, CompositionReference comp) =>
    addMarkerFrb(comp, frame: ui.playheadFrame.value);

Future<void> _compSettings(BuildContext context, LumitState app) async {
  final comp = context.read<LumitUiState>().selectedComp;
  if (comp == null) return;
  final applied = await showCompSettingsFrb(context: context, comp: comp);
  if (applied) app.notifyDocumentChanged();
}

/// Offer to recover work beside the open project.
///
/// Only meaningful once the project has a path — recovery is about a *file*,
/// and a project that has never been saved has nothing beside it.
Future<void> _recover(BuildContext context, LumitState app) async {
  final path = app.project?.path();
  if (path == null) return;
  await showRecoveryDialogFrb(context: context, state: app, projectPath: path);
}

/// Where the welcome screen's thumbnails come from (K-468). Replaced in a test
/// the way [openExternalLink] is; the application's own is the Viewer's.
Future<Uint8List?> Function() projectThumbnailCapture = captureViewerPicturePng;

/// Keep a picture of the project just saved, for the welcome screen's recent
/// row to show next launch (K-468).
///
/// **Nothing here may cost anybody a save.** It runs after the write has
/// finished and the notice has been posted, and it swallows everything: a
/// boundary that has not painted, a driver that will not read the texture back,
/// a machine with no graphics adapter, a read-only appdata folder. Every one of
/// those costs one row its picture and shows the placeholder instead, which is
/// a state the row is built for.
///
/// **The Viewer first, the engine after.** A Viewer that is up has already
/// painted the frame, so photographing it costs nothing and shows exactly what
/// the user was looking at. A Viewer that is not — a headless save, an After
/// Effects conversion, the welcome screen's own New project card before the
/// shell exists, a workspace with the panel closed — used to end here with no
/// picture at all, which is what made the feature read as removed. [comp] is
/// what the engine draws instead, at [frame].
Future<void> fileProjectThumbnail(String path,
    {CompositionReference? comp, int frame = 0}) async {
  Uint8List? png;
  try {
    png = await projectThumbnailCapture();
  } catch (_) {
    // A boundary that would not photograph is not the end of it: the engine can
    // still draw the composition itself.
  }
  if (png == null && comp != null) {
    try {
      png = await Workspace.compThumbnailPng(comp, frame);
    } catch (_) {}
  }
  // The write swallows its own failures; a read-only appdata folder costs one
  // row its picture and nothing else.
  if (png != null) Workspace.writeThumbnail(path, png);
}

/// Save the project, asking for a location only when there is not one already
/// — or always, for Save as.
///
/// The engine refuses an empty path on a project that has never been saved, so
/// whether to prompt is decided here from `path()` rather than by trying and
/// handling the failure.
///
/// [picker] is the injectable seam a widget test needs: no plugin channel can
/// open a real dialogue in one.
Future<void> saveProjectFrb(
  LumitState app,
  LumitUiState ui, {
  bool forcePicker = false,
  Future<String?> Function()? picker,
}) async {
  final project = app.project;
  if (project == null) return;

  var target = '';
  if (forcePicker || project.path() == null) {
    final picked = await (picker ?? pickProjectSaveLocation)();
    if (picked == null) return;
    target = picked;
  }
  // How the interface is arranged goes into the file, so a project handed to
  // someone else opens the way it was left (K-245). Written here rather than
  // as it changes, because this is the moment it is asked for: recording a
  // panel drag into the document would make moving furniture an unsaved change.
  project.setUiState(uiState: ui.sessionJson());
  try {
    final written = await project.save(path: target);
    app.postNotice(l10n.savedTo(written));
    // Save as gives the project a new path, and the session is filed by path —
    // and the title bar carries the name.
    ui.rememberSession();
    app.refreshWindowTitle();
    // The welcome screen's picture of this project, taken *after* the save and
    // never in front of it (K-468): the save is done, the notice is posted, and
    // taking the picture happens on its own time. The fronted composition at
    // the playhead is what it is a picture of, falling back to the project's
    // first comp — a save made from the welcome screen fronts nothing, and a
    // project's first comp is the one its row is recognised by.
    final comps = app.comps();
    unawaited(fileProjectThumbnail(
      written,
      comp: ui.selectedComp ?? (comps.isEmpty ? null : comps.first.$1),
      frame: ui.playheadFrame.value,
    ));
  } catch (_) {
    // The work is still in the document and the journal; say so calmly and let
    // the user pick somewhere writable.
    app.postNotice(l10n.couldNotSaveProject, error: true);
  }
  app.notifyDocumentChanged();
}

/// The Help ▸ Check for updates row, reading whatever the updater is doing
/// (K-296).
///
/// Built fresh every time the service notifies, which is what makes one row
/// carry the whole sequence: check, offer, download, restart. Disabled while
/// something is in flight — a second press during a check would start a second
/// one, and there is nothing useful for it to do.
MenuEntry updateMenuEntry(
  BuildContext context,
  LumitState app,
  LumitUiState ui, {
  Future<String?> Function()? savePicker,
}) {
  final updates = ui.updates;
  return MenuEntry(
    updates.menuLabel,
    updates.busy
        ? null
        : () => pressUpdateRow(
              context,
              updates: updates,
              notice: app.postNotice,
              projectIsDirty: () => app.project?.isDirty() ?? false,
              saveProject: () => saveProjectFrb(app, ui, picker: savePicker),
            ),
  );
}

// --- The macOS renderer ---------------------------------------------------

/// The same tree as native macOS menus.
///
/// Two Mac conventions are applied here and nowhere else, because they are only
/// conventions there: the application menu leads the bar, carrying About, the
/// system-provided Services/Hide/Quit rows and Settings — so Settings is lifted
/// out of Edit and About out of Help on that platform alone. Ticks are drawn as
/// a leading mark in the label, since Flutter's platform-menu API has no
/// checked state of its own.
///
/// **The tick is a character here and only here.** Flutter's platform-menu API
/// hands macOS a label and nothing else — [PlatformMenuItem] has no checked
/// state and no leading widget — so there is no channel to put a glyph down.
/// The in-app renderer below draws the set's tick like every other menu; this
/// one prefixes `✓ ` to the label and pads the untick to match. The ceiling is
/// exactly that API: when `PlatformMenuItem` gains a checked state, or when
/// Lumit draws its own bar on macOS, this branch loses its reason to exist.
List<PlatformMenuItem> platformMenusFor(
    BuildContext context, List<MenuSection> menus) {
  final keymap = context.read<LumitUiState>().keymap;

  List<PlatformMenuItem> rows(List<MenuEntry> items) {
    final out = <PlatformMenuItem>[];
    var group = <PlatformMenuItem>[];
    void flush() {
      if (group.isEmpty) return;
      out.add(PlatformMenuItemGroup(members: group));
      group = [];
    }

    for (final raw in items) {
      if (raw.isDivider) {
        flush();
        continue;
      }
      // A live row resolves to whatever it currently reads. The Mac menu is
      // rebuilt whenever the bar is, and the bar watches the same object the
      // row does, so this stays in step without the in-app renderer's
      // rebuild-in-place machinery.
      final item = raw.current;
      final label = switch (item.checked) {
        true => '✓ ${item.text}',
        false => '  ${item.text}',
        null => item.text,
      };
      if (item.children case final children?) {
        group.add(PlatformMenu(label: label, menus: rows(children)));
        continue;
      }
      final chord =
          item.action == null ? null : keymap.rawChordFor(item.action!);
      group.add(PlatformMenuItem(
        label: label,
        // A null callback is how the platform menu draws a row disabled.
        onSelected: item.onPressed,
        shortcut: chord == null ? null : activatorForChord(chord),
      ));
    }
    flush();
    return out;
  }

  // Settings and About move into the application menu; the menus they came
  // from lose exactly those rows.
  MenuEntry? take(String title, String label) {
    for (final menu in menus) {
      if (menu.title != title) continue;
      for (final item in menu.items()) {
        if (item.label == label) return item;
      }
    }
    return null;
  }

  final settings = take(l10n.menuEdit, l10n.menuSettings);
  final about = take(l10n.menuHelp, l10n.menuAboutLumit);

  return [
    PlatformMenu(label: 'Lumit', menus: [
      PlatformMenuItemGroup(members: [
        PlatformMenuItem(
            label: l10n.menuAboutLumit, onSelected: about?.onPressed),
      ]),
      PlatformMenuItemGroup(members: [
        if (settings != null)
          PlatformMenuItem(
            label: l10n.menuSettings,
            onSelected: settings.onPressed,
            shortcut:
                activatorForChord(keymap.rawChordFor('app.settings') ?? ''),
          ),
      ]),
      const PlatformMenuItemGroup(members: [
        PlatformProvidedMenuItem(
            type: PlatformProvidedMenuItemType.servicesSubmenu),
      ]),
      const PlatformMenuItemGroup(members: [
        PlatformProvidedMenuItem(type: PlatformProvidedMenuItemType.hide),
        PlatformProvidedMenuItem(
            type: PlatformProvidedMenuItemType.hideOtherApplications),
        PlatformProvidedMenuItem(
            type: PlatformProvidedMenuItemType.showAllApplications),
      ]),
      const PlatformMenuItemGroup(members: [
        PlatformProvidedMenuItem(type: PlatformProvidedMenuItemType.quit),
      ]),
    ]),
    for (final menu in menus)
      PlatformMenu(
        label: menu.title,
        menus: rows([
          for (final item in menu.items())
            if (!identical(item, settings) && !identical(item, about)) item,
        ]),
      ),
  ];
}

// --- The in-app renderer --------------------------------------------------

/// Holds the subscription to a shortcut-request bump — the palette's
/// Ctrl+Shift+P, the console's Ctrl+Space (K-324) — and opens its surface
/// when the notifier fires. Draws nothing; it exists only so the menu bar
/// itself stays a plain stateless widget.
class _RequestHotkey extends StatefulWidget {
  final ValueNotifier<int> requests;
  final VoidCallback onRequested;

  const _RequestHotkey({required this.requests, required this.onRequested});

  @override
  State<_RequestHotkey> createState() => _RequestHotkeyState();
}

class _RequestHotkeyState extends State<_RequestHotkey> {
  @override
  void initState() {
    super.initState();
    widget.requests.addListener(_open);
  }

  @override
  void didUpdateWidget(covariant _RequestHotkey old) {
    super.didUpdateWidget(old);
    if (old.requests != widget.requests) {
      old.requests.removeListener(_open);
      widget.requests.addListener(_open);
    }
  }

  void _open() {
    if (mounted) widget.onRequested();
  }

  @override
  void dispose() {
    widget.requests.removeListener(_open);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => const SizedBox.shrink();
}

/// The heading whose menu is up, and the handle that takes it down.
///
/// While one menu is open the bar is *in menus*: crossing another heading hands
/// over to it rather than making the user click a second time, which is how the
/// bar behaves in every application these menus sit beside. One pair for the
/// whole bar, because only one menu is ever open.
String? _openHeading;
VoidCallback? _closeHeading;

class _MenuButton extends StatelessWidget {
  final String title;
  /// Built when the menu opens, not when the bar does — see [MenuSection].
  final List<MenuEntry> Function() items;
  const _MenuButton({required this.title, required this.items});

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      // Only once a menu is already open: hovering the bar with nothing open
      // must not start dropping menus at a passing pointer.
      onEnter: (_) {
        if (_openHeading != null && _openHeading != title) _open(context);
      },
      child: HouseButton(
        key: ValueKey<String>('menu-$title'),
        frameless: true,
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        onPressed: () => _open(context),
        child: Text(title),
      ),
    );
  }

  void _open(BuildContext context) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    final origin = box.localToGlobal(Offset(0, box.size.height));
    _closeHeading?.call();
    _openHeading = title;
    // Built here, once, rather than inside the popup's builder: the rows are
    // this menu's answer at the moment it was asked for.
    final rows = items();
    showLumitPopup<void>(
      context: context,
      position: origin,
      // The bar underneath has to keep feeling the pointer; that is the whole
      // mechanism of handing over to the next heading.
      hoverThrough: true,
      builder: (close) {
        if (_openHeading == title) _closeHeading = () => close(null);
        return _OpenMenu(
          title: title,
          child: _MenuList(items: rows, close: () => close(null)),
        );
      },
    );
  }
}

/// The open menu itself, which forgets it is open when it goes.
///
/// Told by disposal rather than by the close call, so a menu that goes with its
/// window — a test ending, a reload — leaves the bar out of menus too, rather
/// than with a heading it thinks is still open.
class _OpenMenu extends StatefulWidget {
  final String title;
  final Widget child;
  const _OpenMenu({required this.title, required this.child});

  @override
  State<_OpenMenu> createState() => _OpenMenuState();
}

class _OpenMenuState extends State<_OpenMenu> {
  @override
  Widget build(BuildContext context) => widget.child;

  @override
  void dispose() {
    // Not if another heading has already taken over: this menu's disposal
    // arrives a frame after the one that replaced it opened.
    if (_openHeading == widget.title) {
      _openHeading = null;
      _closeHeading = null;
    }
    super.dispose();
  }
}

class _MenuList extends StatefulWidget {
  final List<MenuEntry> items;
  final VoidCallback close;
  const _MenuList({required this.items, required this.close});

  @override
  State<_MenuList> createState() => _MenuListState();
}

/// Stateful for one reason: a [MenuEntry.toggle] row leaves the menu up, so the
/// list has to redraw itself to show the tick it just changed (K-520).
class _MenuListState extends State<_MenuList> {
  @override
  Widget build(BuildContext context) {
    final items = widget.items;
    final close = widget.close;
    final t = ThemeScope.of(context).theme;
    final keymap = context.read<LumitUiState>().keymap;
    // A tick column only where something in this menu is a toggle, so an
    // ordinary menu is not indented for a mark it never shows.
    final ticks = items.any((i) => i.checked != null);
    // A long menu on a short window would otherwise run off the bottom, where
    // the last items cannot be clicked at all — so it scrolls once it no longer
    // fits. `- 40` leaves the menu bar itself and a margin.
    final maxHeight =
        (MediaQuery.of(context).size.height - 40).clamp(80.0, 1e6);
    return FloatSurface(
      width: 300,
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
                    child: _label(t, item, ticks: ticks, shortcut: null),
                  )
                else if (item.live case final listenable?)
                  // Stays open and redraws in place: the point of a live row is
                  // to watch what pressing it did (K-296).
                  ListenableBuilder(
                    listenable: listenable,
                    builder: (context, _) {
                      final row = item.current;
                      return MenuRow(
                        onPressed: row.onPressed ?? () {},
                        child: _label(t, row, ticks: ticks, shortcut: null),
                      );
                    },
                  )
                else
                  MenuRow(
                    key: item.label == null
                        ? null
                        : ValueKey<String>('menu-row-${item.label}'),
                    onPressed: item.onPressed == null
                        ? close
                        : item.keepsMenuOpen
                            // A toggle stays put and redraws its own tick.
                            ? () {
                                item.onPressed!();
                                if (mounted) setState(() {});
                              }
                            : () {
                                close();
                                item.onPressed!();
                              },
                    child: _label(
                      t,
                      item,
                      ticks: ticks,
                      shortcut: item.action == null
                          ? null
                          : keymap.chordFor(item.action!),
                    ),
                  ),
            ],
          ),
        ),
      ),
    );
  }

  /// One row's contents: the tick column where the menu has toggles, the name,
  /// and the chord it answers to on the right.
  Widget _label(LumitTheme t, MenuEntry item,
      {required bool ticks, required String? shortcut}) {
    final style = item.enabled ? null : t.body.copyWith(color: t.textDisabled);
    return Row(
      children: [
        if (ticks) menuTick(item.checked == true, colour: style?.color),
        Expanded(child: Text(item.text, style: style)),
        if (shortcut != null)
          Padding(
            padding: const EdgeInsets.only(left: 16),
            child: Text(shortcut, style: t.small.copyWith(color: t.textMuted)),
          ),
      ],
    );
  }
}
