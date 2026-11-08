// The right-click menu of one property row, for the row's name and stopwatch,
// and the rows it adds to the value field's own menu. One list for an effect's
// parameters and a layer's transform, in Effect controls and in the Timeline's
// fold-out, so a property answers the same way wherever it is shown.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'keyframe_controls_frb.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

/// What one property row can do from its menu. Anything left null is a row the
/// menu leaves out, because this property could never do it.
class ParamMenu {
  const ParamMenu({
    this.own,
    this.keys,
    this.reset,
    this.canReset = true,
    this.copy,
    this.paste,
    this.setExpression,
    this.removeExpression,
    this.graphPath,
  });

  /// Commands that only this kind of row has, at the top of the menu: a pair's
  /// link, a rotation's order.
  final List<Widget> Function(VoidCallback close)? own;

  /// The row's stopwatch, for adding and removing keys.
  final KeyframeControlsFrb? keys;
  final VoidCallback? reset;

  /// False while the value is already at its default. The row stays in the menu
  /// and greys, as it could apply with a different value.
  final bool canReset;

  /// The value as text, and setting it from text. A number copied here pastes
  /// into any other row or field.
  final String Function()? copy;
  final void Function(String text)? paste;
  final VoidCallback? setExpression;
  final VoidCallback? removeExpression;

  /// The row's path in the Timeline's selection.
  final String? graphPath;
}

/// Pick out the numbers in pasted text, whichever way they were separated.
List<double> numbersIn(String text) => [
      for (final m in RegExp(r'-?\d*\.?\d+(?:[eE][-+]?\d+)?').allMatches(text))
        double.parse(m[0]!),
    ];

MenuRow _row(String key, String label, VoidCallback onPressed,
        {String? action, bool enabled = true}) =>
    MenuRow(
      key: ValueKey<String>(key),
      enabled: enabled,
      onPressed: onPressed,
      child: MenuLabel(label, action: action, enabled: enabled),
    );

/// Add or remove a key at the playhead and stop animating, for this row's
/// keyframes.
List<Widget> paramKeyRows(
    BuildContext context, ParamMenu menu, VoidCallback close) {
  final keys = menu.keys;
  if (keys == null) return const [];
  final playhead =
      Provider.of<LumitUiState>(context, listen: false).playheadFrame;
  final frame = playhead.value;
  return [
    _row(
      'param-menu-key',
      keys.hasKeyAt(frame) ? l10n.tipRemoveKeyframe : l10n.tipAddKeyframe,
      () {
        close();
        keys.toggleKeyAt(playhead.value);
      },
      action: 'keyframe.toggle',
    ),
    if (keys.animated)
      _row('param-menu-stop', l10n.tipStopAnimating, () {
        close();
        keys.toggleAnimatedAt(playhead.value);
      }),
  ];
}

/// Set or remove an expression, and go to the row's curve.
List<Widget> paramLastRows(
    BuildContext context, ParamMenu menu, VoidCallback close) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  return [
    if (menu.setExpression case final set?)
      _row('param-menu-expression', l10n.setExpression, () {
        close();
        noteSetExpression();
        set();
      }, action: 'expression.add'),
    if (menu.removeExpression case final remove?)
      _row('param-menu-no-expression', l10n.removeExpression, () {
        close();
        remove();
      }),
    if (menu.graphPath case final path?)
      _row('param-menu-graph', l10n.paramMenuRevealInGraph, () {
        close();
        ui.requestRevealInGraph(path);
      }),
  ];
}

/// The groups in order, with the menu divider between those that have rows.
List<Widget> menuGroups(List<List<Widget>> groups) {
  final out = <Widget>[];
  for (final group in groups) {
    if (group.isEmpty) continue;
    if (out.isNotEmpty) out.add(const MenuDivider());
    out.addAll(group);
  }
  return out;
}

/// The menu itself, at [at].
void showParamMenu(BuildContext context, Offset at, ParamMenu menu) {
  showMenuAt<void>(
    context: context,
    position: at,
    rows: (close) {
      void done() => close(null);
      return menuGroups([
        menu.own?.call(done) ?? const [],
        [
          if (menu.reset case final reset?)
            _row('param-menu-reset', l10n.reset, () {
              done();
              reset();
            }, enabled: menu.canReset),
        ],
        [
          if (menu.copy case final copy?)
            _row('param-menu-copy', l10n.paramMenuCopyValue, () {
              done();
              Clipboard.setData(ClipboardData(text: copy()));
            }),
          if (menu.paste case final paste?)
            _row('param-menu-paste', l10n.paramMenuPasteValue, () async {
              done();
              final data = await Clipboard.getData(Clipboard.kTextPlain);
              final text = data?.text;
              if (text != null) paste(text);
            }),
        ],
        paramKeyRows(context, menu, done),
        paramLastRows(context, menu, done),
      ]);
    },
  );
}

/// The rows a property adds to its value field's own menu. [withReset] is for a
/// field that knows no default of its own, so the row's Reset stands in.
Widget withParamValueMenu(ParamMenu menu, Widget field,
        {bool withReset = false}) =>
    ValueMenuScope(
      before: (context, close) => [
        if (withReset && menu.reset != null)
          _row('param-menu-field-reset', l10n.reset, () {
            close();
            menu.reset!();
          }, enabled: menu.canReset),
        ...paramKeyRows(context, menu, close),
      ],
      after: (context, close) => paramLastRows(
          context,
          // The field's own Set expression stands for the row's.
          ParamMenu(
            removeExpression: menu.removeExpression,
            graphPath: menu.graphPath,
          ),
          close),
      child: field,
    );
