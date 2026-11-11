// Save graphics template: a name, a group in the library, and whether the
// fonts its text names travel with it. Saved again under its own name, a
// template becomes the library's next version of itself.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/template/library.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'dialog_frame.dart';
import 'settings_account.dart' show askBefore;

/// How wide the labels stand.
const double _labelColumn = 64;

/// Open the Save graphics template dialogue for [comp]. Answers once it has
/// closed, saved or not.
Future<void> saveGraphicsTemplate(
    BuildContext context, CompositionReference comp) async {
  final app = Provider.of<LumitState>(context, listen: false);
  final ui = Provider.of<LumitUiState>(context, listen: false);
  // Read once, as the dialogue opens.
  final ownId = comp.getTemplate()?.id;
  List<BridgeLibraryTemplate> library;
  try {
    library = listTemplates();
  } catch (_) {
    library = const [];
  }
  final saved = ownId == null
      ? null
      : library.where((t) => t.id == ownId).firstOrNull;
  List<BridgeFontUse> fonts;
  try {
    fonts = comp.templateFonts();
  } catch (_) {
    fonts = const [];
  }
  final groups = <String>{
    for (final t in library)
      for (var g = t.group; g.isNotEmpty; g = _parentOf(g)) g,
  }.toList()
    ..sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
  final name = TextEditingController(
      text: saved?.name ?? comp.getSettings().name);
  name.selection =
      TextSelection(baseOffset: 0, extentOffset: name.text.length);
  final group = TextEditingController(text: saved?.group ?? '');
  if (!context.mounted) return;
  final done = await showLumitModal<bool>(
    context: context,
    id: 'save-graphics-template',
    builder: (close) => _SaveTemplate(
      comp: comp,
      ownId: ownId,
      fonts: fonts,
      groups: groups,
      name: name,
      group: group,
      close: close,
    ),
  );
  name.dispose();
  group.dispose();
  if (done ?? false) {
    ui.presetsChanged.value++;
    app.notifyDocumentChanged();
  }
}

String _parentOf(String group) {
  final at = group.lastIndexOf('/');
  return at < 0 ? '' : group.substring(0, at);
}

class _SaveTemplate extends StatefulWidget {
  final CompositionReference comp;

  /// The library's id for this template, null until it has been saved.
  final UuidValue? ownId;
  final List<BridgeFontUse> fonts;
  final List<String> groups;
  final TextEditingController name;
  final TextEditingController group;
  final void Function(bool?) close;

  const _SaveTemplate({
    required this.comp,
    required this.ownId,
    required this.fonts,
    required this.groups,
    required this.name,
    required this.group,
    required this.close,
  });

  @override
  State<_SaveTemplate> createState() => _SaveTemplateState();
}

class _SaveTemplateState extends State<_SaveTemplate> {
  bool _packFonts = false;
  bool _saving = false;

  /// Why the last save failed, said in the footer.
  String? _failed;

  /// Holds Escape while it saves, so the dialogue stays up.
  VoidCallback? _holdEscape;

  /// The library's own groups are parted by `/`, whatever the typing.
  String get _group => widget.group.text
      .trim()
      .replaceAll('\\', '/')
      .split('/')
      .map((part) => part.trim())
      .where((part) => part.isNotEmpty)
      .join('/');

  bool get _canSave => !_saving && widget.name.text.trim().isNotEmpty;

  @override
  void initState() {
    super.initState();
    widget.name.addListener(_changed);
  }

  @override
  void dispose() {
    widget.name.removeListener(_changed);
    _holdEscape?.call();
    super.dispose();
  }

  void _changed() => setState(() => _failed = null);

  void _cancel() {
    if (!_saving) widget.close(false);
  }

  Future<void> _save() async {
    if (!_canSave) return;
    final name = widget.name.text.trim();
    final group = _group;
    final there = templateAt(name: name, group: group);
    if (there != null &&
        there != widget.ownId &&
        !await askBefore(context, l10n.templateReplace(name),
            l10n.gradeLookReplaceYes)) {
      return;
    }
    if (!mounted) return;
    setState(() {
      _saving = true;
      _failed = null;
    });
    _holdEscape = EscapeLadder.register(EscapeRung.dialog, () => true);
    try {
      await widget.comp
          .saveTemplate(name: name, group: group, packFonts: _packFonts);
    } catch (error) {
      _holdEscape?.call();
      _holdEscape = null;
      if (mounted) {
        setState(() {
          _saving = false;
          _failed = l10n.templateSaveFailed('$error');
        });
      }
      return;
    }
    _holdEscape?.call();
    _holdEscape = null;
    widget.close(true);
  }

  void _pickGroup(Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 220,
      rows: (close) => [
        for (final group in ['', ...widget.groups])
          MenuRow(
            key: ValueKey<String>('template-save-group-$group'),
            selected: group == _group,
            onPressed: () {
              close(null);
              setState(() => widget.group.text = group);
            },
            child: Padding(
              padding: EdgeInsetsDirectional.only(
                  start: group.isEmpty ? 0 : '/'.allMatches(group).length * 18),
              child: Text(
                  group.isEmpty
                      ? l10n.graphicsTemplates
                      : group.substring(group.lastIndexOf('/') + 1),
                  overflow: TextOverflow.ellipsis),
            ),
          ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final packable = widget.fonts.any((f) => f.installed);
    return dialogEnter(
      onEnter: _save,
      child: DialogFrame(
        width: 360,
        children: [
          dialogTitleBar(t,
              title: l10n.templateSaveTitle,
              onClose: _cancel,
              keyPrefix: 'template-save'),
          Padding(
            padding: dialogBodyPadding,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                dialogRow(
                  t,
                  l10n.name,
                  HouseTextField(
                    key: const ValueKey('template-save-name'),
                    controller: widget.name,
                    autofocus: true,
                    width: double.infinity,
                    onSubmitted: (_) => _save(),
                  ),
                  labelColumn: _labelColumn,
                ),
                const SizedBox(height: 6),
                dialogRow(
                  t,
                  l10n.templateLibraryGroup,
                  Row(
                    children: [
                      Expanded(
                        child: HouseTextField(
                          key: const ValueKey('template-save-group'),
                          controller: widget.group,
                          hint: l10n.graphicsTemplates,
                          width: double.infinity,
                          onSubmitted: (_) => _save(),
                        ),
                      ),
                      if (widget.groups.isNotEmpty)
                        Builder(
                          builder: (context) => HouseButton(
                            key: const ValueKey('template-save-groups'),
                            frameless: true,
                            small: true,
                            padding: const EdgeInsets.all(3),
                            onPressed: () {
                              final box =
                                  context.findRenderObject() as RenderBox?;
                              if (box == null) return;
                              _pickGroup(box.localToGlobal(
                                  Offset(0, box.size.height)));
                            },
                            child: lumitIcon(LumitIcon.twirlOpen,
                                size: iconSize, color: t.textMuted),
                          ),
                        ),
                    ],
                  ),
                  labelColumn: _labelColumn,
                ),
                const SizedBox(height: 10),
                HouseCheckbox(
                  key: const ValueKey('template-save-pack-fonts'),
                  value: _packFonts && packable,
                  label: l10n.templatePackFonts,
                  onChanged: packable && !_saving
                      ? (v) => setState(() => _packFonts = v)
                      : null,
                ),
                for (final font in widget.fonts)
                  Padding(
                    padding: const EdgeInsetsDirectional.only(
                        start: 22, top: 4),
                    child: Row(
                      children: [
                        Flexible(
                          child: Text(
                            '${font.family} ${font.face}'.trim(),
                            key: ValueKey<String>(
                                'template-save-font-${font.family}-${font.face}'),
                            overflow: TextOverflow.ellipsis,
                            style: t.small.copyWith(
                                color: font.installed
                                    ? t.textSecondary
                                    : t.textMuted),
                          ),
                        ),
                        if (!font.installed)
                          LumitTooltip(
                            message: l10n.templateFontMissing,
                            child: Padding(
                              padding:
                                  const EdgeInsetsDirectional.only(start: 4),
                              child: glyph.LumitIcon(LumitIcons.missing,
                                  size: 12, colour: t.warning),
                            ),
                          ),
                      ],
                    ),
                  ),
              ],
            ),
          ),
          dialogFooter(
            t,
            summary: _failed ?? '',
            keyPrefix: 'template-save',
            actions: [
              HouseButton(
                key: const ValueKey('template-save-cancel'),
                onPressed: _saving ? null : _cancel,
                child: Text(l10n.cancel),
              ),
              HouseButton(
                key: const ValueKey('template-save-ok'),
                primary: true,
                onPressed: _canSave ? _save : null,
                child: Text(_saving ? l10n.templateSaving : l10n.save),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
