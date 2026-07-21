// The application shell: menu bar, docked panels, status line, modals and
// the keyboard shortcut routing — the Flutter counterpart of shell/mod.rs +
// app_update.rs + shortcuts.rs.
//
// Structure note: the ThemeScope sits ABOVE the app's one Overlay so that
// popups inserted into the Overlay (menus, dropdowns, tooltips) still read
// the theme; the shell body is its own StatefulWidget *inside* the overlay's
// initial entry, because an OverlayEntry's builder does not re-run when an
// ancestor's setState fires — the body must own its modal state itself.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../panels/panels.dart';
import '../state/app_state.dart';
import '../state/dock.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';
import 'command_palette.dart';
import 'dock_widget.dart';
import 'menu_bar.dart';
import 'settings_window.dart';
import 'splash.dart';

class LumitShell extends StatelessWidget {
  final Workspace workspace;
  const LumitShell({super.key, required this.workspace});

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: workspace,
      builder: (context, _) => ThemeScope(
        theme: workspace.theme,
        animationLevel: workspace.animationLevel,
        showTooltips: workspace.interface.showTooltips,
        child: Overlay(
          initialEntries: [
            OverlayEntry(
              builder: (context) => _ShellBody(workspace: workspace),
            ),
          ],
        ),
      ),
    );
  }
}

class _ShellBody extends StatefulWidget {
  final Workspace workspace;
  const _ShellBody({required this.workspace});

  @override
  State<_ShellBody> createState() => _ShellBodyState();
}

class _ShellBodyState extends State<_ShellBody> {
  final AppStateStub app = AppStateStub();
  bool settingsOpen = false;
  bool paletteOpen = false;
  bool splashDone = false;
  final ValueNotifier<Panel?> activePanel = ValueNotifier(null);
  final FocusNode _rootFocus = FocusNode(debugLabel: 'lumit-shell');

  Workspace get ws => widget.workspace;

  @override
  void dispose() {
    activePanel.dispose();
    _rootFocus.dispose();
    super.dispose();
  }

  /// The global shortcut set (docs/flutter-port/02 §5), with the "never
  /// steal typing" gate: if the focused node is an editable text, stand down.
  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final focused = FocusManager.instance.primaryFocus;
    if (focused != null && focused.context?.widget is EditableText) {
      return KeyEventResult.ignored;
    }
    if (settingsOpen || paletteOpen) {
      if (event.logicalKey == LogicalKeyboardKey.escape) {
        setState(() {
          settingsOpen = false;
          paletteOpen = false;
        });
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }

    final pressed = HardwareKeyboard.instance;
    final ctrl = pressed.isControlPressed || pressed.isMetaPressed;
    final shift = pressed.isShiftPressed;
    final alt = pressed.isAltPressed;
    final key = event.logicalKey;

    bool handled = true;
    if (ctrl && shift && key == LogicalKeyboardKey.keyZ) {
      app.engine('Redo');
    } else if (ctrl && key == LogicalKeyboardKey.keyZ) {
      app.engine('Undo');
    } else if (ctrl && key == LogicalKeyboardKey.keyS) {
      app.engine('Save');
    } else if (ctrl && key == LogicalKeyboardKey.comma) {
      setState(() => settingsOpen = true);
    } else if (ctrl && shift && key == LogicalKeyboardKey.keyP) {
      setState(() => paletteOpen = true);
    } else if (ctrl && key == LogicalKeyboardKey.keyD) {
      if (app.selectedLayer != null) app.engine('Duplicate layer');
    } else if (shift && key == LogicalKeyboardKey.f3) {
      app.toggleGraphMode();
    } else if (key == LogicalKeyboardKey.space) {
      app.togglePlay();
    } else if (key == LogicalKeyboardKey.keyK) {
      if (app.playing) app.togglePlay();
    } else if (key == LogicalKeyboardKey.keyL) {
      if (!app.playing) app.togglePlay();
    } else if (key == LogicalKeyboardKey.keyJ ||
        key == LogicalKeyboardKey.arrowLeft) {
      app.stepFrame(-1);
    } else if (key == LogicalKeyboardKey.arrowRight) {
      app.stepFrame(1);
    } else if (key == LogicalKeyboardKey.home) {
      app.goToFrame(0);
    } else if (key == LogicalKeyboardKey.end) {
      app.goToFrame(app.previewFrameCount);
    } else if (key == LogicalKeyboardKey.keyB) {
      app.engine('Work area in at playhead');
    } else if (key == LogicalKeyboardKey.keyN) {
      app.engine('Work area out at playhead');
    } else if (key == LogicalKeyboardKey.delete ||
        key == LogicalKeyboardKey.backspace) {
      app.engine('Delete selected keyframes or layer');
    } else if (key == LogicalKeyboardKey.equal ||
        key == LogicalKeyboardKey.add) {
      app.zoomTimeline(1.4);
    } else if (key == LogicalKeyboardKey.minus) {
      app.zoomTimeline(1 / 1.4);
    } else if (key == LogicalKeyboardKey.backslash) {
      app.zoomTimelineFit();
    } else if (key == LogicalKeyboardKey.bracketLeft) {
      if (app.selectedLayer != null) {
        app.engine(
            alt ? 'Trim layer in to playhead' : 'Move layer in to playhead');
      } else {
        handled = false;
      }
    } else if (key == LogicalKeyboardKey.bracketRight) {
      if (app.selectedLayer != null) {
        app.engine(
            alt ? 'Trim layer out to playhead' : 'Move layer out to playhead');
      } else {
        handled = false;
      }
    } else if (event.character == '*') {
      // Layout-independent, like the egui text-event read.
      app.engine('Add marker at playhead');
    } else {
      handled = false;
    }
    return handled ? KeyEventResult.handled : KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    return Focus(
      focusNode: _rootFocus,
      autofocus: true,
      onKeyEvent: _onKey,
      child: Stack(
        children: [
          Column(
            children: [
              LumitMenuBar(
                app: app,
                workspace: ws,
                onOpenSettings: () => setState(() => settingsOpen = true),
                onOpenPalette: () => setState(() => paletteOpen = true),
              ),
              Expanded(
                child: DockWidget(
                  root: ws.dock,
                  buildPanel: (context, panel) =>
                      buildPanelBody(context, panel, app),
                  onLayoutChanged: ws.save,
                  activePanel: activePanel,
                  onPopOut: (panel) => app.setNotice(
                      '${panel.title}: pop out arrives with multi-window support'),
                ),
              ),
              _StatusLine(app: app),
            ],
          ),
          if (settingsOpen)
            SettingsWindow(
              workspace: ws,
              app: app,
              onClose: () => setState(() => settingsOpen = false),
            ),
          if (paletteOpen)
            CommandPalette(
              commands: paletteCommands(
                app: app,
                workspace: ws,
                openSettings: () => setState(() {
                  paletteOpen = false;
                  settingsOpen = true;
                }),
              ),
              onClose: () => setState(() => paletteOpen = false),
            ),
          if (!splashDone)
            SplashOverlay(
              onDone: () => setState(() => splashDone = true),
            ),
        ],
      ),
    );
  }
}

/// The status line: quiet notices left, genuine errors in the error tint
/// (docs/15 §10), the export-progress slot right.
class _StatusLine extends StatelessWidget {
  final AppStateStub app;
  const _StatusLine({required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: app,
      builder: (context, _) => Container(
        height: 22,
        color: t.surface2,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Row(
          children: [
            if (app.errorNotice != null)
              Text(app.errorNotice!, style: t.small.copyWith(color: t.error))
            else if (app.notice != null)
              Text(app.notice!, style: t.small),
            const Spacer(),
            Text('Flutter frontend — phase F0', style: t.small),
          ],
        ),
      ),
    );
  }
}
