// The right-click menu of one property row. One list for an effect's parameters,
// a layer's transform and every other row with a stopwatch, in Effect controls
// and in the Timeline's fold-out, so a property answers the same way wherever it
// is shown and wherever on the row it is pressed.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/menu_animation_frb.dart' show addExpressionRow;
import '../widgets/controls.dart';
import 'drive_chain.dart';
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
    this.drive,
  });

  /// What Drive with can wire onto this row. Null on a row that takes no
  /// driver at all.
  final DriveRow? drive;

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

/// The row as a socket on its layer's graph, for Drive with: which instance
/// and port it is, whether it takes a colour, and the number it shows with
/// its range, so a drive starts from the value that was there.
///
/// A transform row has no socket, so it leaves [instance] null and gives
/// [wiggle], which writes the wobble as an expression instead.
class DriveRow {
  final LayerReference layer;
  final UuidValue? instance;
  final String port;
  final bool colour;
  /// The number the row shows, read when the menu opens rather than at
  /// every build.
  final double Function() current;
  final double low;
  final double high;
  final VoidCallback? wiggle;

  const DriveRow({
    required this.layer,
    this.instance,
    this.port = '',
    this.colour = false,
    this.current = _nought,
    this.low = 0,
    this.high = 100,
    this.wiggle,
  });
}

double _nought() => 0;

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
        {String? action, bool enabled = true, bool tickColumn = false}) =>
    MenuRow(
      key: ValueKey<String>(key),
      enabled: enabled,
      onPressed: onPressed,
      child: MenuLabel(label,
          action: action, enabled: enabled, ticked: tickColumn ? false : null),
    );

/// Add or remove a key at the playhead and stop animating, for this row's
/// keyframes. [tickColumn] keeps the words in line with a menu whose other
/// rows carry a tick.
List<Widget> paramKeyRows(
    BuildContext context, ParamMenu menu, VoidCallback close,
    {bool tickColumn = false}) {
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
      tickColumn: tickColumn,
    ),
    if (keys.animated)
      _row('param-menu-stop', l10n.tipStopAnimating, () {
        close();
        keys.toggleAnimatedAt(playhead.value);
      }, tickColumn: tickColumn),
  ];
}

/// Set or remove an expression, and go to the row's curve.
List<Widget> paramLastRows(
    BuildContext context, ParamMenu menu, VoidCallback close,
    {bool tickColumn = false}) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  return [
    if (menu.setExpression case final set?)
      _row('param-menu-expression', l10n.menuAddExpression, () {
        close();
        noteSetExpression();
        set();
      }, action: 'expression.add', tickColumn: tickColumn),
    if (menu.removeExpression case final remove?)
      _row('param-menu-no-expression', l10n.removeExpression, () {
        close();
        remove();
      }, tickColumn: tickColumn),
    if (menu.graphPath case final path?)
      _row('param-menu-graph', l10n.paramMenuRevealInGraph, () {
        close();
        ui.requestRevealInGraph(path);
      }, tickColumn: tickColumn),
  ];
}

/// Drive with ▸: the drivers this row can take, each staged and wired as one
/// undo step, and Remove driver while a wire feeds the row. Read from the
/// graph as the menu opens, never in a build.
List<Widget> _driveRows(
    BuildContext context, DriveRow drive, VoidCallback done) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final app = Provider.of<LumitState>(context, listen: false);
  void changed() {
    app.notifyDocumentChanged();
    ui.model.refresh();
  }

  // A transform row: the wobble as an expression, and nothing else fits.
  if (drive.instance == null) {
    final wiggle = drive.wiggle;
    if (wiggle == null) return const [];
    return [
      SubmenuRow(
        key: const ValueKey<String>('param-menu-drive'),
        closeParent: done,
        submenu: (dismiss) => FloatSurface(
          width: 150,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _row('param-menu-drive-wiggle', engineLabel('Wiggle'), () {
                dismiss();
                wiggle();
              }),
            ],
          ),
        ),
        child: MenuLabel(l10n.paramMenuDriveWith),
      ),
    ];
  }

  final BridgeLayerGraph graph;
  try {
    graph = drive.layer.getGraph();
  } catch (_) {
    return const [];
  }
  final node = graphNodeWithId(graph, drive.instance!);
  // A clip's or a group's effect has no box on the layer's graph.
  if (node == null) return const [];
  final target = BridgeInputRef.param(node: node, port: drive.port);
  final wired = wiredInto(graph, target) != null;

  void stage(DriveKind kind, {UuidValue? listenTo}) {
    final ok = stageDrive(
      layer: drive.layer,
      target: target,
      kind: kind,
      current: drive.current(),
      low: drive.low,
      high: drive.high,
      listenTo: listenTo,
      colour: drive.colour,
    );
    if (!ok) {
      sayDriveRefused(context);
      return;
    }
    changed();
    if (kind == DriveKind.expression) {
      editDriveExpression(context, drive.layer, target);
    }
  }

  // What a sound driver listens to: the mix, or one layer that can carry
  // sound.
  final sources = [
    for (final e in ui.model.heldLayers)
      if (const {
        BridgeLayerKind.footage,
        BridgeLayerKind.audio,
        BridgeLayerKind.sequence,
        BridgeLayerKind.precomp,
        BridgeLayerKind.bus,
      }.contains(e.info.kind))
        (e.info.name, e.layer.internallayerId),
  ];
  Widget listening(String key, String label, DriveKind kind, VoidCallback close) =>
      SubmenuRow(
        key: ValueKey<String>('param-menu-drive-$key'),
        closeParent: close,
        submenu: (dismiss) => FloatSurface(
          width: 180,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _row('param-menu-drive-$key-comp', l10n.driveWholeComposition, () {
                dismiss();
                stage(kind);
              }),
              if (sources.isNotEmpty) const MenuDivider(),
              for (final (name, id) in sources)
                _row('param-menu-drive-$key-$id', name, () {
                  dismiss();
                  stage(kind, listenTo: id);
                }),
            ],
          ),
        ),
        child: MenuLabel(label),
      );

  return [
    SubmenuRow(
      key: const ValueKey<String>('param-menu-drive'),
      closeParent: done,
      submenu: (dismiss) => FloatSurface(
        width: 160,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (!drive.colour) ...[
              _row('param-menu-drive-wiggle', engineLabel('Wiggle'), () {
                dismiss();
                stage(DriveKind.wiggle);
              }),
              listening('audio', engineLabel('Audio level'),
                  DriveKind.audioLevel, dismiss),
              listening('beat', engineLabel('Beat trigger'),
                  DriveKind.beatTrigger, dismiss),
            ],
            if (drive.colour)
              _row('param-menu-drive-cycle', engineLabel('Colour cycle'), () {
                dismiss();
                stage(DriveKind.colourCycle);
              }),
            _row('param-menu-drive-expression', '${engineLabel('Expression')}…',
                () {
              dismiss();
              stage(DriveKind.expression);
            }),
            if (wired) ...[
              const MenuDivider(),
              _row('param-menu-drive-remove', l10n.driveRemove, () {
                dismiss();
                if (removeDriveFrom(drive.layer, target)) {
                  changed();
                } else {
                  sayDriveRefused(context);
                }
              }),
            ],
          ],
        ),
      ),
      child: MenuLabel(l10n.paramMenuDriveWith),
    ),
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
        _row('param-menu-expression', l10n.menuAddExpression, () {
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
          child: MenuLabel(l10n.menuAddExpression),
        ),
      if (menu.drive case final drive?) ..._driveRows(context, drive, done),
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
