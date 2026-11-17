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
import '../widgets/colour_picker.dart'
    show ColourClipboard, PickedColour, numbersOnly;
import '../widgets/controls.dart';
import 'drive_chain.dart';
import 'effect_param_row_frb.dart' show newExpressionLanguage;
import 'graph_channels.dart' show graphChannels;
import 'keyframe_controls_frb.dart';
import 'property_link.dart';
import '../state/expression_language.dart' show PropertyRef;
import '../state/template_controls.dart' show ControlRow;
import 'control_rows.dart' show withSelectedControlRows;
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
    this.colour,
    this.pasteColour,
    this.setExpression,
    this.axisExpressions = const [],
    this.removeExpression,
    this.editExpression,
    this.convertExpression,
    this.graphPath,
    this.host,
    this.onCurves = false,
    this.drive,
    this.linkTo,
    this.linkFrom,
    this.control,
  });

  /// This row as one a control of the composition's graphics template can
  /// name, for Add to controls. Null on a row the engine cannot name.
  final ControlRow? control;

  /// This row as something another row can follow, for Copy link: what the
  /// drop of a link drag on it would name.
  final PropertyRef? Function(PropertyLink link)? linkTo;

  /// This row as the one that follows, for Paste link: what its link mark
  /// would drag.
  final PropertyLink? linkFrom;

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
  final RowKeys? keys;
  final VoidCallback? reset;

  /// False while the value is already at its default. The row stays in the menu
  /// and greys, as it could apply with a different value.
  final bool canReset;

  /// The value as text, and setting it from text. A number copied here pastes
  /// into any other row or field.
  final String Function()? copy;
  final void Function(String text)? paste;

  /// A colour row's colour and setting it, for the clipboard rows every
  /// colour well has, in place of Copy value and Paste value.
  final PickedColour Function()? colour;
  final void Function(PickedColour colour)? pasteColour;
  final VoidCallback? setExpression;

  /// Set expression on a row of several axes, one each: an expression is worked
  /// out for one number.
  final List<({String axis, VoidCallback set})> axisExpressions;
  final VoidCallback? removeExpression;

  /// The row's expression, opened in its editor.
  final VoidCallback? editExpression;

  /// The expression baked into a key a frame, across the layer.
  final VoidCallback? convertExpression;

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

/// The numbers in pasted text for a row of [axes] axes, or null when they don't
/// fit it. One number goes on every axis.
List<double>? pastedAxes(String text, int axes) {
  final n = numbersOnly(text);
  if (n == null) return null;
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

/// Rows the place a property row is drawn in adds to its menu, after Reset:
/// a copy's card adds Set as default to the rows of a control.
class ParamRowExtras extends InheritedWidget {
  final List<Widget> Function(VoidCallback close) rows;
  const ParamRowExtras({super.key, required this.rows, required super.child});

  static List<Widget> Function(VoidCallback close)? of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<ParamRowExtras>()?.rows;

  @override
  bool updateShouldNotify(ParamRowExtras old) => false;
}

/// The menu of a row whose value is simply the scalars under its stopwatch [keys]:
/// a mask's opacity, a stroke's start, a volume. [defaults] is what Reset puts
/// back, one per axis, on a row that has one.
ParamMenu scalarRowMenu(
  BuildContext context,
  KeyframeControlsFrb keys, {
  List<double>? defaults,
  void Function(BuildContext, Offset, List<Widget> Function(VoidCallback))? host,
  ControlRow? control,
}) {
  final scalars = keys.scalars;
  final held = scalars.any((s) => s is BridgeScalar_Expression);
  final path = ParamRowPath.of(context);
  final playhead =
      Provider.of<LumitUiState>(context, listen: false).playheadFrame;
  return ParamMenu(
    host: host,
    control: control,
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

/// Copy value and Paste value, for a menu that lays out the row's rows itself.
/// [clip] is the clipboard's text as the menu opened, '' for nothing, and
/// Paste value greys when the row can't take it.
List<Widget> paramCopyRows(
    BuildContext context, ParamMenu menu, VoidCallback close,
    {bool tickColumn = false, String? clip}) {
  // A paste on a row with several keys picked moves them all, as typing does.
  final edit = context.getInheritedWidgetOfExactType<ValueEditScope>();
  // Copy value writes one number an axis, so the row's own value counts them.
  final axes = numbersOnly(menu.copy?.call() ?? '')?.length ?? 1;
  final canPaste = clip == null || pastedAxes(clip, axes) != null;
  return [
    if (menu.copy case final copy?)
      _row('param-menu-copy', l10n.paramMenuCopyValue, () {
        close();
        Clipboard.setData(ClipboardData(text: copy()));
      }, tickColumn: tickColumn),
    if (menu.paste case final paste?)
      _row('param-menu-paste', l10n.paramMenuPasteValue, () async {
        close();
        final data = await Clipboard.getData(Clipboard.kTextPlain);
        final text = data?.text;
        // The value the row already has makes no undo step.
        if (text == null || (edit == null && text == menu.copy?.call())) return;
        edit == null ? paste(text) : edit.run(true, () => paste(text));
      }, enabled: canPaste, tickColumn: tickColumn),
  ];
}

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
    if (menu.editExpression case final edit?)
      _row('param-menu-edit-expression', l10n.expressionEdit, () {
        close();
        edit();
      }, tickColumn: tickColumn),
    if (menu.convertExpression case final convert?)
      _row('param-menu-convert-expression', l10n.expressionConvertToKeyframes,
          () {
        close();
        convert();
      }, tickColumn: tickColumn),
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

/// Add to controls, or Remove from controls on a row that is one: a group of
/// its own in the row's menu. Left out where the engine cannot name the row,
/// and inside a node graph. In the Timeline the entry takes the other
/// selected rows with it when the row is one of them, as one undo step.
List<Widget> paramControlRows(
    BuildContext context, ParamMenu menu, VoidCallback close,
    {bool tickColumn = false}) {
  final control = menu.control;
  if (control == null) return const [];
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final controls = ui.templateControls;
  if (!controls.offers(control.comp)) return const [];
  final isControl = controls.controlFor(control.row) != null;
  // Only the Timeline has rows selected. Elsewhere it is the row pressed.
  final rows = ParamRowPath.of(context) == null
      ? [control.row, ...control.also]
      : withSelectedControlRows(ui, control.row);
  return [
    _row(
      'param-menu-control',
      isControl ? l10n.controlsRemove : l10n.controlsAdd,
      () {
        close();
        isControl ? controls.remove(rows) : controls.add(rows);
      },
      action: isControl ? null : 'template.controls.add',
      tickColumn: tickColumn,
    ),
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

/// Paste link: the copied row, written as this row's expression and opened
/// in the editor, as a link dragged here would be. Greyed with nothing copied
/// that this row can name.
Widget _pasteLink(BuildContext context, PropertyLink link, VoidCallback done) {
  final ref = copiedLink?.call(link);
  final got = ref == null ? null : link.textFor(ref, newExpressionLanguage(context));
  return _row('param-menu-paste-link', l10n.paramMenuPasteLink, () {
    done();
    if (got case (final text, final language)) link.write?.call(text, language);
  }, enabled: got != null);
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

/// The menu itself, at [at]. The clipboard is read first, so a paste with
/// nothing to take greys.
Future<void> showParamMenu(
    BuildContext context, Offset at, ParamMenu menu) async {
  final clip = menu.paste != null || menu.pasteColour != null
      ? await ColourClipboard.read()
      : null;
  if (!context.mounted) return;
  List<Widget> rows(VoidCallback close) => _rows(context, menu, close, clip);
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

List<Widget> _rows(
    BuildContext context, ParamMenu menu, VoidCallback done, String? clip) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
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
      ...?ParamRowExtras.of(context)?.call(done),
    ],
    [
      ...paramCopyRows(context, menu, done, clip: clip ?? ''),
      if (menu.colour case final colour?) ...[
        _row('param-menu-copy-colour', l10n.swatchCopyColour, () {
          done();
          ColourClipboard.copy(colour(), linear: true);
        }),
        if (menu.pasteColour case final paste?)
          _row('param-menu-paste-colour', l10n.swatchPasteColour, () {
            done();
            final got = ColourClipboard.from(clip, linear: true);
            if (got != null) paste(got);
          }, enabled: ColourClipboard.from(clip, linear: true) != null),
        _row('param-menu-copy-hex', l10n.swatchCopyHex, () {
          done();
          ColourClipboard.copyHex(colour(), linear: true);
        }),
      ],
      if (menu.linkTo case final to?)
        _row('param-menu-copy-link', l10n.paramMenuCopyLink, () {
          done();
          copiedLink = to;
        }),
      if (menu.linkFrom case final link?)
        _pasteLink(context, link, done),
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
          // The chord puts one on every axis of the row.
          child: MenuLabel(l10n.menuAddExpression, action: 'expression.add'),
        ),
      if (menu.drive case final drive?) ..._driveRows(context, drive, done),
      if (menu.editExpression case final edit?)
        _row('param-menu-edit-expression', l10n.expressionEdit, () {
          done();
          edit();
        }),
      if (menu.convertExpression case final convert?)
        _row('param-menu-convert-expression',
            l10n.expressionConvertToKeyframes, () {
          done();
          convert();
        }),
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
    paramControlRows(context, menu, done),
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
