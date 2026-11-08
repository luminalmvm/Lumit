// Settings → Appearance → Customise: every colour in the theme, editable.
//
// **What it edits.** The theme you are looking at, live. Opening seeds every
// row from the current theme — built-in or one of your own — and each change
// applies to the running app immediately, because a colour you cannot see
// against the rest of the interface is a colour you cannot judge. Closing
// without saving puts back exactly what was there.
//
// **What it saves.** A named custom theme: the name, the light-or-dark base,
// and the colours. Saving from a built-in scheme asks for a name and makes a
// new theme; saving while one of your own is selected updates that one, which
// is what "select it, customise it, save" ought to mean. **Save a copy…**
// writes the edits down under a new name instead, so a theme can be
// branched without first being overwritten.
//
// One colour is not offered: the Viewer's surround. It is strictly neutral by
// spec (docs/15-DESIGN §2.1/§11) because a grade cannot be judged against a
// tinted surround — the one place the interface's own taste has to give way.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../main.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../theme/custom_theme.dart';
import '../theme/theme.dart';
import '../theme/theme_tokens.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'settings_rows.dart';
import 'theme_name_dialog.dart';

/// Open the editor over [ui]'s current theme. Returns when it closes.
Future<void> showThemeEditorFrb(BuildContext context, LumitUiState ui) =>
    showLumitModal<void>(
      context: context,
      id: 'theme-editor',
      initialSize: const Size(460, 560),
      minSize: const Size(360, 300),
      builder: (close) => _ThemeEditor(ui: ui, onClose: () => close(null)),
    );

class _ThemeEditor extends StatefulWidget {
  final LumitUiState ui;
  final VoidCallback onClose;

  const _ThemeEditor({required this.ui, required this.onClose});

  @override
  State<_ThemeEditor> createState() => _ThemeEditorState();
}

class _ThemeEditorState extends State<_ThemeEditor> {
  /// The colours as edited. Seeded from the theme on screen when the editor
  /// opened, which is what "the colours displayed are the ones you use" means.
  late Map<String, Color> _colours;

  /// What to put back on discard: the whole appearance selection, not just the
  /// colours, because previewing rebuilds the live theme.
  late final String? _wasCustom;
  late final LumitColorScheme _wasScheme;

  bool _dirty = false;

  @override
  void initState() {
    super.initState();
    final workspace = widget.ui.workspace;
    _wasCustom = workspace.customThemeName;
    _wasScheme = workspace.colorScheme;
    _colours = Map.of(tokensOf(widget.ui.theme));
  }

  /// The theme the rows describe: the base this is built over, with the edits
  /// on top. Also what the app is showing while the editor is open.
  LumitTheme get _edited => applyTokens(_baseTheme(), _colours);

  LumitTheme _baseTheme() {
    final workspace = widget.ui.workspace;
    final scheme =
        workspace.activeCustomTheme?.baseScheme ?? workspace.colorScheme;
    return LumitTheme.forScheme(scheme, workspace.themeShape);
  }

  /// Show the edit in the running app, so a colour is judged where it is used.
  void _preview() {
    widget.ui.workspace.previewTheme(_edited);
    setState(() => _dirty = true);
  }

  /// Put the appearance back the way it was found.
  void _revert() {
    final workspace = widget.ui.workspace;
    workspace.customThemeName = _wasCustom;
    workspace.colorScheme = _wasScheme;
    workspace.clearPreview();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final groups = themeTokenGroups;

    return DialogFrame(
      children: [
        dialogTitleBar(
          t,
          title: l10n.themeEditorTitle,
          subject: widget.ui.workspace.customThemeName ?? '',
          onClose: _close,
          keyPrefix: 'theme-editor',
        ),
        Expanded(
          child: ListView(
            key: const ValueKey('theme-editor-body'),
            padding: EdgeInsets.zero,
            children: [
              for (final (i, group) in groups.indexed)
                settingsSection(t, group, first: i == 0, [
                  for (final token
                      in themeTokens.where((token) => token.group == group))
                    _tokenRow(t, token),
                ]),
              const SizedBox(height: 8),
            ],
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'theme-editor',
          actions: [
            HouseButton(
              key: const ValueKey('theme-editor-cancel'),
              onPressed: () {
                _revert();
                widget.onClose();
              },
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('theme-editor-save-copy'),
              onPressed: _saveCopy,
              child: Text(l10n.themeSaveACopy),
            ),
            HouseButton(
              key: const ValueKey('theme-editor-save'),
              primary: true,
              onPressed: _save,
              child: Text(l10n.save),
            ),
          ],
        ),
      ],
    );
  }

  Widget _tokenRow(LumitTheme t, ThemeToken token) {
    final colour = _colours[token.key] ?? token.read(_baseTheme());
    return settingsRow(
      t,
      token.label,
      '',
      GestureDetector(
        onSecondaryTapDown: (press) =>
            _tokenMenu(token, colour, press.globalPosition),
        child: _Swatch(
          key: ValueKey('theme-token-${token.key}'),
          colour: colour,
          onPicked: (picked) {
            _colours[token.key] = picked;
            _preview();
          },
        ),
      ),
    );
  }

  /// One colour's menu: move it between rows, or back to the theme underneath.
  Future<void> _tokenMenu(ThemeToken token, Color colour, Offset at) async {
    final pasted = parseHex(
        (await Clipboard.getData(Clipboard.kTextPlain))?.text?.trim() ?? '');
    if (!mounted) return;
    void set(Color c) {
      _colours[token.key] = c;
      _preview();
    }

    final base = token.read(_baseTheme());
    await showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('theme-token-copy'),
          onPressed: () {
            close(null);
            Clipboard.setData(ClipboardData(text: formatHex(colour)));
          },
          child: Text(l10n.menuCopy),
        ),
        MenuRow(
          key: const ValueKey('theme-token-paste'),
          enabled: pasted != null,
          onPressed: () {
            close(null);
            if (pasted != null) set(pasted);
          },
          child: Text(l10n.menuPaste),
        ),
        const MenuDivider(),
        MenuRow(
          key: const ValueKey('theme-token-reset'),
          enabled: colour != base,
          onPressed: () {
            close(null);
            set(base);
          },
          child: Text(l10n.reset),
        ),
      ],
    );
  }

  /// Save: name it if it has none yet, otherwise update the theme in place.
  Future<void> _save() async {
    final workspace = widget.ui.workspace;
    var name = workspace.customThemeName;
    if (name == null) {
      name = await askThemeName(context,
          title: l10n.themeNameThis, suggested: l10n.themeDefaultName);
      if (name == null || !mounted) return;
      name = workspace.availableThemeName(name);
    }
    _saveAs(name);
  }

  /// Save a copy: the edits as they stand, under a new name, leaving whatever
  /// was open where it was. What "I like this theme, but…" wants —
  /// without it, the only way to branch a theme was to overwrite it and undo
  /// the edits by hand.
  Future<void> _saveCopy() async {
    final workspace = widget.ui.workspace;
    final asked = await askThemeName(
      context,
      title: l10n.themeNameTheCopy,
      suggested: workspace.availableThemeName(l10n.themeCopySuffix(
          workspace.customThemeName ?? workspace.themeChoice.label)),
    );
    if (asked == null || !mounted) return;
    _saveAs(workspace.availableThemeName(asked));
  }

  /// Write the edits down under [name] and select it. The one place that
  /// builds the stored theme, so Save and Save a copy cannot disagree about
  /// what a save is.
  void _saveAs(String name) {
    final workspace = widget.ui.workspace;
    workspace.clearPreview();
    workspace.saveCustomTheme(CustomTheme(
      name: name,
      mode: _edited.mode,
      colours: Map.of(_colours),
    ));
    setState(() => _dirty = false);
  }

  /// Closing with unsaved edits asks rather than assuming. Either answer is a
  /// real choice — discarding is not a failure state — so neither is styled
  /// as the dangerous one.
  Future<void> _close() async {
    if (!_dirty) {
      _revert();
      widget.onClose();
      return;
    }
    final keep = await askLumitChoice<bool>(
      context: context,
      title: l10n.themeEditorTitle,
      question: l10n.themeSaveChanges,
      answers: [(l10n.discard, false), (l10n.save, true)],
    );
    if (!mounted || keep == null) return; // Dismissed: stay in the editor.
    if (keep) {
      await _save();
      if (!mounted) return;
    } else {
      _revert();
    }
    widget.onClose();
  }
}

/// The colour cell: the colour, and a click that opens the picker on it.
class _Swatch extends StatelessWidget {
  final Color colour;
  final ValueChanged<Color> onPicked;

  const _Swatch({super.key, required this.colour, required this.onPicked});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () async {
        final box = context.findRenderObject()! as RenderBox;
        final origin = box.localToGlobal(Offset.zero);
        // Live: the editor's own preview repaints as the colour changes, so
        // the theme is judged on the interface rather than in a swatch.
        await showColourPicker(
          context: context,
          position: origin + Offset(0, box.size.height + 4),
          initial: PickedColour.of(colour),
          // A theme colour is a display colour: eight bits a channel, and a
          // hex is the same value said another way.
          scale: ColourScale.bytes,
          onCommit: (picked) => onPicked(picked.clipped),
          onPreview: (picked) => onPicked(picked.clipped),
        );
      },
      child: HoverMark(
        swells: true,
        builder: (_, lit) => Container(
          width: 56,
          height: 22,
          decoration: BoxDecoration(
            color: colour,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(
                color: Color.lerp(t.hairlineStrong, t.textPrimary, 0.7 * lit)!),
          ),
        ),
      ),
    );
  }
}
