// The command palette (docs/07-UI-SPEC §12), ported from
// shell/command_palette.rs: Ctrl+Shift+P opens a modal search over the
// app-wide command list; Enter or a click runs the selection.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../state/app_state.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

class PaletteCommand {
  final String label;

  /// Hidden search aliases (e.g. the export command's
  /// "render output video mp4", command_palette.rs:158).
  final String aliases;
  final VoidCallback run;

  const PaletteCommand(this.label, this.run, {this.aliases = ''});

  bool matches(String query) {
    final q = query.trim().toLowerCase();
    if (q.isEmpty) return true;
    final hay = '${label.toLowerCase()} ${aliases.toLowerCase()}';
    // Every whitespace-separated term must appear somewhere.
    return q.split(RegExp(r'\s+')).every(hay.contains);
  }
}

/// The shipped command list, mirroring the egui palette's coverage: global
/// actions, layer adds, colour schemes, Settings and export.
List<PaletteCommand> paletteCommands({
  required AppStateStub app,
  required Workspace workspace,
  required VoidCallback openSettings,
}) =>
    [
      PaletteCommand('Save project', app.save),
      PaletteCommand('Undo', app.undo),
      PaletteCommand('Redo', app.redo),
      PaletteCommand('New project', app.newProject),
      PaletteCommand('Open project…', app.openProject),
      PaletteCommand('Import footage…', app.importFootage),
      PaletteCommand('New composition', app.newComposition),
      PaletteCommand('Add solid layer', () => app.engine('Add solid layer')),
      PaletteCommand('Add text layer', () => app.engine('Add text layer')),
      PaletteCommand('Add camera layer', () => app.engine('Add camera layer')),
      PaletteCommand(
          'Add adjustment layer', () => app.engine('Add adjustment layer')),
      PaletteCommand(
          'Add sequence layer', () => app.engine('Add sequence layer')),
      PaletteCommand('Add marker at playhead',
          () => app.engine('Add marker at playhead')),
      PaletteCommand('Export comp…', () => app.engine('Export comp'),
          aliases: 'render output video mp4'),
      PaletteCommand('Reset workspace', workspace.resetWorkspaceLayout),
      PaletteCommand('Open Settings', openSettings),
      for (final s in LumitColorScheme.values)
        PaletteCommand('Colour scheme: ${s.label}',
            () => workspace.setScheme(s)),
    ];

class CommandPalette extends StatefulWidget {
  final List<PaletteCommand> commands;
  final VoidCallback onClose;

  const CommandPalette({
    super.key,
    required this.commands,
    required this.onClose,
  });

  @override
  State<CommandPalette> createState() => _CommandPaletteState();
}

class _CommandPaletteState extends State<CommandPalette> {
  final TextEditingController _query = TextEditingController();
  final FocusNode _focus = FocusNode();
  int _sel = 0;

  List<PaletteCommand> get _filtered =>
      [for (final c in widget.commands) if (c.matches(_query.text)) c];

  @override
  void initState() {
    super.initState();
    // The search field grabs focus on open, like palette_focus.
    WidgetsBinding.instance.addPostFrameCallback((_) => _focus.requestFocus());
    _query.addListener(() => setState(() => _sel = 0));
  }

  @override
  void dispose() {
    _query.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _run(PaletteCommand c) {
    widget.onClose();
    c.run();
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final list = _filtered;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      widget.onClose();
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.arrowDown) {
      setState(() => _sel = (_sel + 1).clamp(0, list.isEmpty ? 0 : list.length - 1));
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.arrowUp) {
      setState(() => _sel = (_sel - 1).clamp(0, list.isEmpty ? 0 : list.length - 1));
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.enter) {
      if (_sel < list.length) _run(list[_sel]);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final list = _filtered;
    return Stack(
      children: [
        Positioned.fill(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: widget.onClose,
            child: Container(color: t.modalBackdrop),
          ),
        ),
        Align(
          alignment: const Alignment(0, -0.6),
          child: Container(
            width: 420,
            constraints: const BoxConstraints(maxHeight: 320),
            decoration: BoxDecoration(
              color: t.surface3,
              borderRadius: BorderRadius.circular(t.tokens.floatRadius),
              border: Border.all(color: t.hairline),
              boxShadow: t.floatShadow,
            ),
            padding: const EdgeInsets.all(8),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Container(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
                  decoration: BoxDecoration(
                    color: t.surface0,
                    borderRadius:
                        BorderRadius.circular(t.tokens.controlRadius),
                    border: Border.all(color: t.hairlineStrong),
                  ),
                  child: Focus(
                    onKeyEvent: _onKey,
                    child: EditableText(
                      controller: _query,
                      focusNode: _focus,
                      style: t.bodyPrimary,
                      cursorColor: t.accent,
                      backgroundCursorColor: t.surface2,
                      selectionColor: t.accent.withValues(alpha: 0.5),
                    ),
                  ),
                ),
                const SizedBox(height: 6),
                Flexible(
                  child: ListView.builder(
                    shrinkWrap: true,
                    itemCount: list.length,
                    itemBuilder: (context, i) => MenuRow(
                      selected: i == _sel,
                      onPressed: () => _run(list[i]),
                      child: Text(list[i].label),
                    ),
                  ),
                ),
                if (list.isEmpty)
                  Padding(
                    padding: const EdgeInsets.all(8),
                    child: Text('No matching command', style: t.small),
                  ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}
