// The right-click menu of one property row. One list for an effect's parameters,
// a layer's transform and every other row with a stopwatch, in Effect controls
// and in the Timeline's fold-out, so a property answers the same way wherever it
// is shown and wherever on the row it is pressed.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_animation_frb.dart' show addExpressionRow;
import '../widgets/controls.dart';
import 'graph_channels.dart' show graphChannels;
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
    this.axisExpressions = const [],
    this.removeExpression,
    this.graphPath,
    this.host,
    this.onCurves = false,
  });

  /// [setExpression] works on the picked property's curve, so it's left out
  /// on a row the graph has no curve for.
  final bool onCurves;

  /// Commands that only this kind of row has, at the top of the menu: a pair's
  /// link, a rotation's order.
  final List<Widget> Function(VoidCallback close)? own;

  /// A menu of the row's own that opens instead and takes these rows after its
  /// own: Retime's.
  final void Function(BuildContext context, Offset at,
      List<Widget> Function(VoidCallback close) rows)? host;

  /// The row's stopwatch, for adding and removing keyframes.
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

  /// Set expression on a row of several axes, one each: an expression is worked
  /// out for one number.
  final List<({String axis, VoidCallback set})> axisExpressions;
  final VoidCallback? removeExpression;

  /// The row's path in the Timeline's selection.
  final String? graphPath;
}

/// Pick out the numbers in pasted text, whichever way they were separated.
List<double> numbersIn(String text) => [
      for (final m in RegExp(r'-?\d*\.?\d+(?:[eE][-+]?\d+)?').allMatches(text))
        double.parse(m[0]!),
    ];

/// The numbers in pasted text for a row of [axes] axes, or null when they don't
/// fit it. One number goes on every axis.
List<double>? pastedAxes(String text, int axes) {
  final n = numbersIn(text);
  if (n.length == 1) return List.filled(axes, n.first);
  return n.length < axes ? null : n.sublist(0, axes);
}

/// Numbers as Copy value writes them, with no more digits than mean anything.
String copiedAxes(Iterable<double> values) => values
    .map((v) => double.parse(v.toStringAsFixed(4)).toString())
    .join(', ');

/// The X, Y and Z of a row's axes, by place.
String axisName(int i) => switch (i) {
      0 => l10n.viewportAxisX,
      1 => l10n.viewportAxisY,
      _ => l10n.viewportAxisZ,
    };

/// A row of the Timeline's fold-out says which property it is here, so its menu
/// can reach the graph and the commands that act on the picked property.
class ParamRowPath extends InheritedWidget {
  final String path;
  const ParamRowPath({super.key, required this.path, required super.child});

  static String? of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<ParamRowPath>()?.path;

  @override
  bool updateShouldNotify(ParamRowPath old) => false;
}

/// The menu of a row whose value is simply the scalars under its stopwatch [keys]:
/// a mask's opacity, a stroke's start, a volume. [defaults] is what Reset puts
/// back, one per axis, on a row that has one.
ParamMenu scalarRowMenu(
  BuildContext context,
  KeyframeControlsFrb keys, {
  List<double>? defaults,
  void Function(BuildContext, Offset, List<Widget> Function(VoidCallback))? host,
}) {
  final scalars = keys.scalars;
  final held = scalars.any((s) => s is BridgeScalar_Expression);
  final path = ParamRowPath.of(context);
  final playhead =
      Provider.of<LumitUiState>(context, listen: false).playheadFrame;
  return ParamMenu(
    host: host,
    onCurves: true,
    keys: held ? null : keys,
    reset: defaults == null
        ? null
        : () => keys.onWrite([for (final d in defaults) BridgeScalar.static_(d)]),
    canReset: defaults == null ||
        [
          for (final (i, s) in scalars.indexed)
            s is! BridgeScalar_Static || (s.field0 - defaults[i]).abs() > 1e-5,
        ].any((differs) => differs),
    copy: held ? null : () => copiedAxes(keys.valuesAtFrame(playhead.value)),
    paste: held
        ? null
        : (text) {
            final n = pastedAxes(text, scalars.length);
            if (n != null) keys.writeValuesAt(n, playhead.value);
          },
    // A press on a row in the fold-out has already picked it, and these work
    // on the picked property, as the Animation menu's own row does.
    setExpression: path == null || held || scalars.length != 1
        ? null
        : () => addExpressionRow(
                context,
                Provider.of<LumitState>(context, listen: false),
                Provider.of<LumitUiState>(context, listen: false))
            .press(),
    removeExpression: !held || scalars.length != 1
        ? null
        : () => keys.onWrite([
              BridgeScalar.static_(keys.valuesAtFrame(playhead.value).first),
            ]),
    graphPath: held ? null : path,
  );
}

MenuRow _row(String key, String label, VoidCallback onPressed,
        {String? action, bool enabled = true}) =>
    MenuRow(
      key: ValueKey<String>(key),
      enabled: enabled,
      onPressed: onPressed,
      child: MenuLabel(label, action: action, enabled: enabled),
    );

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
  List<Widget> rows(VoidCallback close) => _rows(context, menu, close);
  if (menu.host case final host?) {
    host(context, at, rows);
    return;
  }
  showMenuAt<void>(
    context: context,
    position: at,
    rows: (close) => rows(() => close(null)),
  );
}

List<Widget> _rows(BuildContext context, ParamMenu menu, VoidCallback done) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  // A paste on a row with several keys picked moves them all, as typing does.
  final edit = context.getInheritedWidgetOfExactType<ValueEditScope>();
  final playhead = ui.playheadFrame;
  final keys = menu.keys;
  final frame = playhead.value;
  // A row the graph has no curve for can't go there, and the Animation menu's
  // expression row, which works on curves, can't reach it either.
  final path = menu.graphPath;
  final hasCurve = path != null &&
      graphChannels(layers: ui.model.layers, selected: [path]).isNotEmpty;
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
          if (text == null) return;
          edit == null ? paste(text) : edit.run(true, () => paste(text));
        }),
    ],
    [
      if (keys != null) ...[
        _row(
          'param-menu-key',
          keys.hasKeyAt(frame) ? l10n.tipRemoveKeyframe : l10n.tipAddKeyframe,
          () {
            done();
            keys.toggleKeyAt(playhead.value);
          },
          action: 'keyframe.toggle',
        ),
        if (keys.animated)
          _row('param-menu-stop', l10n.tipStopAnimating, () {
            done();
            keys.toggleAnimatedAt(playhead.value);
          }),
      ],
    ],
    [
      if (menu.setExpression case final set? when hasCurve || !menu.onCurves)
        _row('param-menu-expression', l10n.setExpression, () {
          done();
          noteSetExpression();
          set();
        }, action: 'expression.add'),
      if (menu.axisExpressions.isNotEmpty)
        SubmenuRow(
          key: const ValueKey<String>('param-menu-expression'),
          closeParent: done,
          submenu: (dismiss) => FloatSurface(
            width: 110,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final (i, axis) in menu.axisExpressions.indexed)
                  _row('param-menu-expression-$i', axis.axis, () {
                    dismiss();
                    noteSetExpression();
                    axis.set();
                  }),
              ],
            ),
          ),
          child: MenuLabel(l10n.setExpression),
        ),
      if (menu.removeExpression case final remove?)
        _row('param-menu-no-expression', l10n.removeExpression, () {
          done();
          remove();
        }),
      if (path != null && hasCurve)
        _row('param-menu-graph', l10n.paramMenuRevealInGraph, () {
          done();
          ui.requestRevealInGraph(path);
        }),
    ],
  ]);
}

/// A property row, answering a right press with its one menu wherever it lands:
/// the name, the stopwatch, a value well, the space between them.
class ParamRowMenu extends StatelessWidget {
  final ParamMenu menu;
  final Widget child;
  const ParamRowMenu({super.key, required this.menu, required this.child});

  @override
  Widget build(BuildContext context) => ValueMenuScope(
        open: (context, at) => showParamMenu(context, at, menu),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapUp: (d) =>
              showParamMenu(context, d.globalPosition, menu),
          child: child,
        ),
      );
}
