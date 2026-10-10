// The Expressions panel: the user's saved expressions, and an editor that
// puts one on the selected properties.
//
// The library lives in the settings file, so it follows the user from project
// to project. Apply writes the way Animation ▸ Add expression does, onto the
// property rows picked in the Timeline.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_animation_frb.dart' show selectedChannels;
import '../state/file_dialogs.dart';
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
  final TextEditingController _name = TextEditingController();
  final TextEditingController _code = ExpressionTextEditingController();

  /// The saved expression the editor was loaded from, by name.
  String? _picked;

  @override
  void initState() {
    super.initState();
    // The list filters as you type, and the buttons follow the two fields.
    for (final field in [_search, _name, _code]) {
      field.addListener(_redraw);
    }
  }

  void _redraw() => setState(() {});

  @override
  void dispose() {
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
    _name.text = name;
    _code.text = ui.workspace.savedExpressions[name] ?? '';
    setState(() => _picked = name);
  }

  /// Clear the editor, starting from the selected property's expression when
  /// it has one, which is how an expression already in the project is saved.
  void _new(LumitUiState ui) {
    final from = _targets(ui).firstOrNull?.scalar;
    _name.clear();
    _code.text = from is BridgeScalar_Expression ? from.field0 : '';
    setState(() => _picked = null);
  }

  void _save(LumitUiState ui) {
    final name = _name.text.trim();
    ui.workspace.saveExpression(name, _code.text);
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
        channel: BridgeScalar.expression(_code.text),
    });
    app.notifyDocumentChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final app = Provider.of<LumitState>(context, listen: false);
    final ui = Provider.of<LumitUiState>(context);
    final saved = ui.workspace.savedExpressions;
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
                      GestureDetector(
                        key: ValueKey<String>('expr-item-$name'),
                        behavior: HitTestBehavior.opaque,
                        // On the down stroke, so the second press of a
                        // double-click applies what the first one loaded.
                        onTapDown: (_) => _pick(ui, name),
                        onDoubleTap: () => _apply(app, ui),
                        child: Container(
                          height: 20,
                          color: name == _picked ? t.selectionFill : null,
                          padding: const EdgeInsets.symmetric(horizontal: 10),
                          alignment: Alignment.centerLeft,
                          child: Text(name,
                              style: t.body, overflow: TextOverflow.ellipsis),
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
            child: HouseTextField(
              key: const ValueKey('expr-code'),
              controller: _code,
              multiline: true,
              topAlign: true,
              style: t.mono,
              hint: l10n.expressionHint,
              width: double.infinity,
            ),
          ),
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
              // Scrolls sideways when docked narrow, as the preset bar does.
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
              ),
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
