// Settings → Appearance → Customise: every colour in the theme, editable
// (K-202).
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
// (K-298) writes the edits down under a new name instead, so a theme can be
// branched without first being overwritten.
//
// One colour is not offered: the Viewer's surround. It is strictly neutral by
// spec (docs/15-DESIGN §2.1/§11) because a grade cannot be judged against a
// tinted surround — the one place the interface's own taste has to give way.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../main.dart';
import '../theme/custom_theme.dart';
import '../theme/theme.dart';
import '../theme/theme_tokens.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'theme_name_dialog.dart';

/// Open the editor over [ui]'s current theme. Returns when it closes.
Future<void> showThemeEditorFrb(BuildContext context, LumitUiState ui) =>
    showLumitModal<void>(
      context: context,
      id: 'theme-editor',
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

    return FloatSurface(
      width: 560,
      child: SizedBox(
        height: 520,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(14, 10, 10, 6),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      widget.ui.workspace.customThemeName == null
                          ? l10n.themeEditorTitle
                          : l10n.themeEditorTitleNamed(
                              '${widget.ui.workspace.customThemeName}'),
                      style: t.bodyPrimary,
                    ),
                  ),
                  HouseButton(
                    key: const ValueKey('theme-editor-save-copy'),
                    small: true,
                    frameless: true,
                    onPressed: _saveCopy,
                    child: Text(l10n.themeSaveACopy),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('theme-editor-save'),
                    small: true,
                    onPressed: _save,
                    child: Text(l10n.save),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('theme-editor-close'),
                    small: true,
                    frameless: true,
                    onPressed: _close,
                    child: Text(l10n.close),
                  ),
                ],
              ),
            ),
            Container(height: 1, color: t.hairline),
            Expanded(
              child: ListView(
                key: const ValueKey('theme-editor-body'),
                padding: const EdgeInsets.fromLTRB(14, 8, 14, 14),
                children: [
                  for (final group in groups) ...[
                    Padding(
                      padding: const EdgeInsets.fromLTRB(2, 6, 0, 4),
                      child: Text(group,
                          style: t.small.copyWith(color: t.textMuted)),
                    ),
                    Container(
                      decoration: BoxDecoration(
                        color: t.surface1,
                        borderRadius:
                            BorderRadius.circular(t.tokens.floatRadius),
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (final token in themeTokens
                              .where((token) => token.group == group))
                            _tokenRow(t, token),
                        ],
                      ),
                    ),
                  ],
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _tokenRow(LumitTheme t, ThemeToken token) {
    final colour = _colours[token.key] ?? token.read(_baseTheme());
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 7),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(token.label, style: t.body),
                Padding(
                  padding: const EdgeInsets.only(top: 2),
                  child: Text(token.description,
                      style: t.small.copyWith(color: t.textMuted)),
                ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          _Swatch(
            key: ValueKey('theme-token-${token.key}'),
            colour: colour,
            onPicked: (picked) {
              _colours[token.key] = picked;
              _preview();
            },
          ),
        ],
      ),
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
  /// was open where it was (K-298). What "I like this theme, but…" wants —
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
    final keep = await showLumitModal<bool>(
      context: context,
      builder: (close) => FloatSurface(
        width: 340,
        child: Padding(
          padding: const EdgeInsets.all(14),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(l10n.themeSaveChanges,
                  style: ThemeScope.of(context).theme.body),
              const SizedBox(height: 12),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  HouseButton(
                    key: const ValueKey('theme-editor-discard'),
                    small: true,
                    frameless: true,
                    onPressed: () => close(false),
                    child: Text(l10n.discard),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('theme-editor-save-on-close'),
                    small: true,
                    // The default action (K-319): Enter keeps the work.
                    primary: true,
                    autofocus: true,
                    onPressed: () => close(true),
                    child: Text(l10n.save),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
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
      child: Container(
        width: 56,
        height: 22,
        decoration: BoxDecoration(
          color: colour,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.hairlineStrong),
        ),
      ),
    );
  }
}
