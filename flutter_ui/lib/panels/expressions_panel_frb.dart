// The Expressions panel: the user's saved expressions, and an editor that
// puts one on the selected properties.
//
// The library lives in the settings file, so it follows the user from project
// to project. Apply writes the way Animation â–¸ Add expression does, onto the
// property rows picked in the Timeline.
//
// The editor also follows the Timeline. Pick a property that has an
// expression and the editor shows it, and what is typed is written back on
// clicking away. That holds while the editor is empty or still showing a
// property's own text. A saved expression picked from the list, or a draft
// typed for one, stays where it is until New.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_animation_frb.dart' show selectedChannels;
import '../state/comp_time.dart';
import '../state/dock.dart' show Panel;
import '../state/expression_language.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/autofill.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show ExpressionTextEditingController;
import 'graph_channels.dart';
import 'graph_edits.dart';

class ExpressionsPanelFrb extends StatefulWidget {
  const ExpressionsPanelFrb({super.key});

  @override
  State<ExpressionsPanelFrb> createState() => _ExpressionsPanelFrbState();
}

class _ExpressionsPanelFrbState extends State<ExpressionsPanelFrb> {
  final TextEditingController _search = TextEditingController();
  final FocusNode _searchFocus = FocusNode();
  LumitUiState? _boundUi;
  final TextEditingController _name = TextEditingController();
  final ExpressionTextEditingController _code =
      ExpressionTextEditingController();

  /// The saved expression the editor was loaded from, by name.
  String? _picked;

  /// The language the text in the editor is written in. It is saved with the
  /// text and applied with it.
  BridgeExpressionLanguage _language = BridgeExpressionLanguage.rhai;

  /// The property the editor is showing, when it is showing one. What is
  /// typed is written to it on clicking away.
  GraphChannel? _linked;

  /// The text and language last read from a property. The editor follows the
  /// selection while it still holds them untouched, or holds nothing.
  String? _loaded;
  BridgeExpressionLanguage? _loadedLanguage;

  /// Why the text gives no number, and the text it was asked about. Asked
  /// again when the text changes and not on every frame.
  String? _fault;
  String? _faultOf;

  @override
  void initState() {
    super.initState();
    // The list filters as you type, and the buttons follow the two fields.
    for (final field in [_search, _name, _code]) {
      field.addListener(_redraw);
    }
    // Ctrl+F asks the focused panel for its search box.
    _boundUi = Provider.of<LumitUiState>(context, listen: false);
    _boundUi!.panelSearchRequest.addListener(_onSearchRequested);
    _language = _boundUi!.workspace.defaultExpressionLanguage;
    _boundUi!.selectedProperties.addListener(_follow);
    _boundUi!.model.addListener(_follow);
    _follow();
  }

  /// Show the picked property's expression, when the editor is free to.
  void _follow() {
    if (!mounted) return;
    GraphChannel? driven;
    for (final channel in _targets(_boundUi!)) {
      if (channel.scalar is BridgeScalar_Expression) {
        driven = channel;
        break;
      }
    }
    // Moving off a property keeps what was typed for it. Keeping it reads
    // the model again, which comes back through here and does the rest.
    if (_linked != null && driven?.id != _linked!.id && _commitLinked()) {
      return;
    }
    final scalar = driven?.scalar;
    if (driven == null || scalar is! BridgeScalar_Expression) {
      if (_linked != null) setState(() => _linked = null);
      return;
    }
    final untouched = _code.text.isEmpty ||
        (_code.text == _loaded && _language == _loadedLanguage);
    final same = _code.text == scalar.field0 && _language == scalar.field1;
    if (!untouched && !same) {
      // Something else is being written here. It is still this property's
      // when it was this property's before.
      setState(() => _linked = _linked?.id == driven!.id ? driven : null);
      return;
    }
    setState(() {
      _linked = driven;
      _loaded = scalar.field0;
      _loadedLanguage = scalar.field1;
      _language = scalar.field1;
    });
    if (_code.text != scalar.field0) _code.text = scalar.field0;
  }

  /// Write what was typed to the property it was typed for, and say whether
  /// anything was written. Nothing when nothing changed, and a blank changes
  /// nothing.
  bool _commitLinked() {
    final channel = _linked;
    if (channel == null || _code.text.trim().isEmpty) return false;
    if (_code.text == _loaded && _language == _loadedLanguage) return false;
    _loaded = _code.text;
    _loadedLanguage = _language;
    commitChannelEdits(
        {channel: BridgeScalar.expression(_code.text, _language)});
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    _boundUi!.model.refresh();
    return true;
  }

  void _onSearchRequested() {
    if (!mounted) return;
    if (_boundUi?.searchRequestIsFor(Panel.expressions) ?? false) {
      _searchFocus.requestFocus();
    }
  }

  void _redraw() => setState(() {});

  @override
  void dispose() {
    _boundUi?.panelSearchRequest.removeListener(_onSearchRequested);
    _boundUi?.selectedProperties.removeListener(_follow);
    _boundUi?.model.removeListener(_follow);
    _searchFocus.dispose();
    _search.dispose();
    _name.dispose();
    _code.dispose();
    super.dispose();
  }

  /// The property rows an expression can go on. A mask's shape row holds a
  /// path rather than a number, so it is left out.
  List<GraphChannel> _targets(LumitUiState ui) => [
        for (final channel in selectedChannels(ui))
          if (!channel.isMaskPath) channel,
      ];

  void _pick(LumitUiState ui, String name) {
    _commitLinked();
    _name.text = name;
    _code.text = ui.workspace.savedExpressions[name] ?? '';
    setState(() {
      _picked = name;
      _language = ui.workspace.savedExpressionLanguage(name);
      // A saved expression is not the property's, so it stops following.
      _linked = null;
      _loaded = null;
    });
  }

  /// Clear the editor, starting from the selected property's expression when
  /// it has one, which is how an expression already in the project is saved.
  void _new(LumitUiState ui) {
    _commitLinked();
    _name.clear();
    _code.clear();
    setState(() {
      _picked = null;
      _linked = null;
      _loaded = null;
      _language = ui.workspace.defaultExpressionLanguage;
    });
    _follow();
  }

  /// What the text comes to at the playhead, or why it comes to nothing,
  /// worked out about the property it is for. Named when the editor is
  /// showing that property's own.
  Widget _readout(LumitTheme t, LumitUiState ui) {
    final on = _linked ?? _targets(ui).firstOrNull;
    final comp = ui.selectedComp;
    if (on == null || comp == null || _code.text.trim().isEmpty) {
      return const SizedBox.shrink();
    }
    final time = timeOfFrame(comp, ui.playheadFrame.value);
    final layer = on.entry.layer;
    final asked = '${on.id}\n${_language.name}\n${_code.text}';
    if (asked != _faultOf) {
      _faultOf = asked;
      _fault = expressionFault(
          expression: _code.text, language: _language, time: time, layer: layer);
    }
    final fault = _fault;
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 0, 8, 4),
      child: Text.rich(
        TextSpan(children: [
          if (_linked != null)
            TextSpan(
              text: '${on.label}  ',
              style: t.small.copyWith(color: t.textMuted),
            ),
          fault != null
              ? TextSpan(text: fault, style: t.small.copyWith(color: t.error))
              : TextSpan(
                  text: '= ${sampleScalarWithContext(
                    scalar: BridgeScalar_Expression(_code.text, _language),
                    time: time,
                    layer: layer,
                  ).toStringAsPrecision(6)}',
                  style: t.mono.copyWith(color: t.textMuted),
                ),
        ]),
        key: const ValueKey('expr-readout'),
        maxLines: 2,
        overflow: TextOverflow.ellipsis,
      ),
    );
  }

  void _save(LumitUiState ui) {
    final name = _name.text.trim();
    ui.workspace.saveExpression(name, _code.text, _language);
    setState(() => _picked = name);
  }

  /// Write every saved expression to one file, to hand to someone.
  Future<void> _export(LumitState app, LumitUiState ui) async {
    final path = await pickExpressionsSaveLocation();
    if (path == null) return;
    try {
      await File(path).writeAsString(ui.workspace.encodeExpressions());
      app.postNotice(l10n.expressionsExported);
    } catch (_) {
      app.postNotice(l10n.workspaceFileUnwritable, error: true);
    }
  }

  /// Add the expressions in a file to the saved ones.
  Future<void> _import(LumitState app, LumitUiState ui) async {
    final path = await pickExpressionsToOpen();
    if (path == null) return;
    int? count;
    try {
      count = ui.workspace.importExpressions(await File(path).readAsString());
    } catch (_) {
      count = null;
    }
    if (count == null) {
      app.postNotice(l10n.expressionsFileNotExpressions, error: true);
      return;
    }
    app.postNotice(l10n.expressionsImported(count));
    if (mounted) setState(() {});
  }

  void _apply(LumitState app, LumitUiState ui) {
    final targets = _targets(ui);
    if (targets.isEmpty || _code.text.trim().isEmpty) return;
    commitChannelEdits({
      for (final channel in targets)
        channel: BridgeScalar.expression(_code.text, _language),
    });
    app.notifyDocumentChanged();
    ui.model.refresh();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final app = Provider.of<LumitState>(context, listen: false);
    final ui = Provider.of<LumitUiState>(context);
    final saved = ui.workspace.savedExpressions;
    // The editor is coloured in the language the dropdown shows.
    _code.language = expressionGrammar(_language);
    final needle = _search.text.trim().toLowerCase();
    // A search reads the script as well as the name, so "noise" finds every
    // expression that uses it.
    final names = [
      for (final MapEntry(:key, :value) in saved.entries)
        if (key.toLowerCase().contains(needle) ||
            value.toLowerCase().contains(needle))
          key,
    ]..sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));

    final body = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          height: 26,
          color: t.surface1,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: HouseTextField(
            key: const ValueKey('expr-search'),
            controller: _search,
            focusNode: _searchFocus,
            hint: l10n.searchExpressions,
            width: double.infinity,
          ),
        ),
        Expanded(
          flex: 3,
          child: saved.isEmpty
              ? Center(child: Text(l10n.noExpressionsYet, style: t.small))
              : ListView(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  children: [
                    for (final name in names)
                      HoverFill(
                        faint: true,
                        child: GestureDetector(
                          key: ValueKey<String>('expr-item-$name'),
                          behavior: HitTestBehavior.opaque,
                          // On the down stroke, so the second press of a
                          // double-click applies what the first one loaded.
                          onTapDown: (_) => _pick(ui, name),
                          onDoubleTap: () => _apply(app, ui),
                          child: Container(
                            height: 20,
                            color: name == _picked ? t.selectionFill : null,
                            padding:
                                const EdgeInsets.symmetric(horizontal: 10),
                            alignment: Alignment.centerLeft,
                            child: Text(name,
                                style: t.body,
                                overflow: TextOverflow.ellipsis),
                          ),
                        ),
                      ),
                  ],
                ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(6, 4, 6, 4),
          child: HouseTextField(
            key: const ValueKey('expr-name'),
            controller: _name,
            hint: l10n.name,
            width: double.infinity,
          ),
        ),
        Expanded(
          flex: 2,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(6, 0, 6, 4),
            // Ctrl+Enter keeps the edit without leaving the text.
            child: Focus(
              canRequestFocus: false,
              skipTraversal: true,
              onKeyEvent: (_, event) {
                final keys = HardwareKeyboard.instance;
                if (event is KeyDownEvent &&
                    event.logicalKey == LogicalKeyboardKey.enter &&
                    (keys.isControlPressed || keys.isMetaPressed)) {
                  _commitLinked();
                  return KeyEventResult.handled;
                }
                return KeyEventResult.ignored;
              },
              child: HouseTextField(
                key: const ValueKey('expr-code'),
                controller: _code,
                autofill: ExpressionAutofillGenerator(),
                multiline: true,
                indentOnTab: true,
                topAlign: true,
                style: t.mono,
                hint: l10n.expressionHint,
                width: double.infinity,
                onTapOutside: _commitLinked,
              ),
            ),
          ),
        ),
        // The number moves with the playhead, so only this line listens.
        ListenableBuilder(
          listenable: Listenable.merge(
              [ui.selectedProperties, ui.model, ui.playheadFrame]),
          builder: (context, _) => _readout(t, ui),
        ),
        // Apply follows the Timeline's selection, so only the bar listens.
        ListenableBuilder(
          listenable: Listenable.merge([ui.selectedProperties, ui.model]),
          builder: (context, _) {
            final hasTarget = _targets(ui).isNotEmpty;
            final hasCode = _code.text.trim().isNotEmpty;
            Widget button(String key, String label, VoidCallback? onPressed) =>
                HouseButton(
                  key: ValueKey<String>(key),
                  small: true,
                  frameless: true,
                  onPressed: onPressed,
                  child: Text(label, style: t.small),
                );
            return Container(
              height: 26,
              color: t.surface1,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              // The buttons scroll sideways when docked narrow, as the preset
              // bar does. The language keeps the bar's right-hand end.
              child: Row(children: [
                Expanded(
                    child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: [
                    button('expr-new', l10n.newExpression, () => _new(ui)),
                    button(
                      'expr-save',
                      l10n.save,
                      _name.text.trim().isEmpty || !hasCode
                          ? null
                          : () => _save(ui),
                    ),
                    button(
                      'expr-delete',
                      l10n.delete,
                      _picked == null
                          ? null
                          : () {
                              ui.workspace.deleteExpression(_picked!);
                              setState(() => _picked = null);
                            },
                    ),
                    button(
                      'expr-apply',
                      l10n.apply,
                      hasTarget && hasCode ? () => _apply(app, ui) : null,
                    ),
                    button('expr-import', l10n.menuImport,
                        () => _import(app, ui)),
                    button(
                      'expr-export',
                      l10n.menuExport,
                      ui.workspace.savedExpressions.isEmpty
                          ? null
                          : () => _export(app, ui),
                    ),
                    if (!hasTarget) ...[
                      const SizedBox(width: 10),
                      Text(l10n.selectAProperty,
                          style: t.small.copyWith(color: t.textMuted)),
                    ],
                  ],
                  ),
                )),
                const SizedBox(width: 6),
                SizedBox(
                  width: 92,
                  child: ExpressionLanguagePicker(
                    key: const ValueKey('expr-language'),
                    dense: true,
                    value: _language,
                    onChanged: (language) {
                      setState(() => _language = language);
                      _commitLinked();
                    },
                  ),
                ),
              ]),
            );
          },
        ),
      ],
    );
    // A stack can squeeze a panel to a strip, and below this height the panel
    // scrolls instead of crushing its fields.
    return LayoutBuilder(
      builder: (context, box) => SingleChildScrollView(
        child: SizedBox(
          height: box.maxHeight < 160 ? 160 : box.maxHeight,
          child: body,
        ),
      ),
    );
  }
}
