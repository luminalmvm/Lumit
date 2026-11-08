// The right-click menus of the node canvases: a box, a socket, a wire, a tie
// and the ground. One set of rows, so a box on the layer graph, on a node
// graph composition and inside a shader opens the same menu, and each canvas
// hands over only the commands it has. An effect's box on the layer graph
// opens the effect's own menu with the box's rows added.
//
// A row whose callback is null is left out: the box never could do that
// here. A row that could with a different pick, or a different clipboard, is
// drawn disabled instead.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart' show BridgePortType;
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'effect_commands.dart' show EffectCommands;
import 'timeline_extras_frb.dart' show labelChips, showMenuAt;

/// What a socket's colour means, for its tooltip.
String graphPortWord(BridgePortType type) => switch (type) {
      BridgePortType.image => l10n.graphPortImage,
      BridgePortType.matte => l10n.graphPortMatte,
      BridgePortType.depth => l10n.graphPortDepth,
      BridgePortType.number => l10n.graphPortNumber,
      BridgePortType.colour => l10n.graphPortColour,
      BridgePortType.shape => l10n.graphPortShape,
      BridgePortType.points => l10n.graphPortPoints,
      BridgePortType.audio => l10n.graphPortAudio,
    };

/// One row: the word, the keymap action whose chord it shows, and whether it
/// is there, and live, on this canvas.
class _Row {
  final String key;
  final String text;
  final VoidCallback? run;
  final String? action;
  final bool enabled;
  final bool? ticked;

  /// A flyout instead of a command, built with what closes the whole menu.
  final Widget Function(VoidCallback dismiss)? submenu;
  const _Row(this.key, this.text, this.run,
      {this.action, this.enabled = true, this.ticked, this.submenu});
}

/// The groups' rows with a line between each pair, empty groups dropped.
Future<void> _show(
  BuildContext context,
  Offset at,
  List<List<_Row>> groups,
) {
  final kept = [
    for (final g in groups)
      if (g.any((r) => r.run != null)) [for (final r in g) if (r.run != null) r],
  ];
  final ticks = kept.any((g) => g.any((r) => r.ticked != null));
  return showMenuAt<void>(
    context: context,
    position: at,
    rows: (close) => [
      for (final (i, g) in kept.indexed) ...[
        if (i > 0) const MenuDivider(),
        for (final r in g) _draw(r, () => close(null), ticks: ticks),
      ],
    ],
  );
}

/// One row as the menu draws it. [ticks] keeps a tick column on every row
/// when any row in the menu has a tick, so the names line up.
Widget _draw(_Row r, VoidCallback close, {required bool ticks}) {
  if (r.submenu case final submenu?) {
    return SubmenuRow(
      key: ValueKey<String>('graph-menu-${r.key}'),
      closeParent: close,
      submenu: submenu,
      child: MenuLabel(r.text, ticked: ticks ? false : null),
    );
  }
  return MenuRow(
    key: ValueKey<String>('graph-menu-${r.key}'),
    enabled: r.enabled,
    onPressed: () {
      close();
      r.run!();
    },
    child: MenuLabel(r.text,
        action: r.action,
        enabled: r.enabled,
        ticked: ticks ? (r.ticked ?? false) : null),
  );
}

/// A box's menu. [bypassed] is null on a box with no enable tick, [expanded]
/// null on one with no twirl, and [enterLabel] names what a double-click
/// opens, where there is an inside to open.
///
/// An effect's box on the layer's graph hands over its [effect] and opens the
/// effect's own menu, the one its headings open, with the box's rows in their
/// places. The effect then answers for Bypass, the clipboard, the reveal, the
/// manual and Delete, so those callbacks are not asked for.
///
/// A [locked] layer refuses every edit, so a row that would make one greys,
/// and the Colour flyout is left out, as the effect's Move and Presets are.
Future<void> showGraphBoxMenu(
  BuildContext context,
  Offset at, {
  EffectCommands? effect,
  bool? bypassed,
  VoidCallback? onBypass,
  VoidCallback? onRename,
  bool? expanded,
  VoidCallback? onExpand,
  String? enterLabel,
  VoidCallback? onEnter,
  VoidCallback? onAddNode,
  VoidCallback? onCopy,
  VoidCallback? onPaste,
  bool canPaste = true,
  VoidCallback? onDuplicate,
  VoidCallback? onGroup,
  VoidCallback? onSaveGroup,
  bool canSaveGroup = true,
  VoidCallback? onUngroup,
  bool canUngroup = true,
  bool? viewHere,
  VoidCallback? onViewHere,
  VoidCallback? onReveal,
  VoidCallback? onManual,
  VoidCallback? onDisconnectAll,
  bool? collapsed,
  VoidCallback? onCollapse,
  int colour = 0,
  ValueChanged<int>? onColour,
  VoidCallback? onDelete,
  bool locked = false,
}) {
  final open = !locked;
  // What a box does on its canvas: the same rows, in the same places, on an
  // effect's box and on any other.
  final own = [
    // The socket menu's pair, so the twirl has one name wherever it is
    // pressed.
    _Row(
        'expand',
        (expanded ?? false) ? l10n.graphHideSockets : l10n.graphShowSockets,
        onExpand,
        enabled: open),
    if (enterLabel != null) _Row('enter', enterLabel, onEnter),
    _Row('add-node', l10n.graphAddNode, onAddNode,
        action: 'console.open', enabled: open),
    _Row('disconnect-all', l10n.graphDisconnectAll, onDisconnectAll,
        enabled: open),
  ];
  final arrange = [
    _Row('group', l10n.graphGroup, onGroup,
        action: 'layer.group', enabled: open),
    _Row('save-group', l10n.graphSaveGroup, onSaveGroup,
        enabled: canSaveGroup && open),
    _Row('ungroup', l10n.graphUngroup, onUngroup,
        action: 'layer.ungroup', enabled: canUngroup && open),
  ];
  // The Viewer's "at" chip, from the box itself. Ticked while the picture
  // stops here.
  final view =
      _Row('view-here', l10n.graphViewHere, onViewHere, ticked: viewHere);
  final look = [
    _Row('collapse', (collapsed ?? false) ? l10n.expand : l10n.collapse,
        onCollapse,
        enabled: open),
    // The Timeline's label chips; the first, the quiet one, takes the
    // colour away.
    _Row('colour', l10n.graphBoxColour,
        onColour == null || locked ? null : () {},
        submenu: (dismiss) => FloatSurface(
              child: labelChips(ThemeScope.of(context).theme,
                  keyPrefix: 'graph-menu-colour', selected: colour,
                  onPick: (i) {
                dismiss();
                onColour?.call(i);
              }),
            )),
  ];
  if (effect != null) {
    // Bypass gives the effect's menu a tick column.
    List<Widget> draw(List<_Row> rows, VoidCallback close) => [
          for (final r in rows)
            if (r.run != null) _draw(r, close, ticks: true),
        ];
    effect.showMenu(context, at,
        onStartRename: onRename,
        box: (close) => (
              own: draw(own, close),
              arrange: draw(arrange, close),
              view: draw([view], close),
              look: draw(look, close),
            ));
    return Future.value();
  }
  return _show(context, at, [
    [
      _Row('bypass', l10n.fxBypass, onBypass, ticked: bypassed, enabled: open),
      _Row('rename', l10n.rename, onRename,
          action: 'effect.rename', enabled: open),
    ],
    own,
    [
      _Row('copy', l10n.menuCopy, onCopy, action: 'edit.copy'),
      _Row('paste', l10n.menuPaste, onPaste,
          action: 'edit.paste', enabled: canPaste && open),
      _Row('duplicate', l10n.menuDuplicate, onDuplicate,
          action: 'effect.duplicate', enabled: open),
    ],
    arrange,
    [_Row('manual', l10n.fxMenuManual, onManual)],
    [view, _Row('reveal', l10n.graphRevealEffect, onReveal)],
    look,
    [
      _Row('delete', l10n.delete, onDelete,
          action: 'edit.delete.selection', enabled: open),
    ],
  ]);
}

/// A socket's menu: take its wire off, and open or shut the box's unused
/// sockets.
Future<void> showGraphSocketMenu(
  BuildContext context,
  Offset at, {
  VoidCallback? onDisconnect,
  bool? expanded,
  VoidCallback? onExpand,
}) =>
    _show(context, at, [
      [_Row('disconnect', l10n.graphDisconnect, onDisconnect)],
      [
        _Row(
            'expand',
            (expanded ?? false) ? l10n.graphHideSockets : l10n.graphShowSockets,
            onExpand),
      ],
    ]);

/// A wire's menu.
Future<void> showGraphWireMenu(
  BuildContext context,
  Offset at, {
  VoidCallback? onInsert,
  VoidCallback? onReroute,
  VoidCallback? onDelete,
}) =>
    _show(context, at, [
      [
        _Row('insert', l10n.graphInsertNode, onInsert),
        _Row('reroute', l10n.graphAddReroute, onReroute),
      ],
      [_Row('delete', l10n.delete, onDelete)],
    ]);

/// A tie's menu, where a stationary right-click used to take the tie away
/// with no way to ask first.
Future<void> showGraphTieMenu(
  BuildContext context,
  Offset at, {
  required VoidCallback onDelete,
}) =>
    _show(context, at, [
      [_Row('delete', l10n.delete, onDelete)],
    ]);

/// The ground's menu.
Future<void> showGraphCanvasMenu(
  BuildContext context,
  Offset at, {
  VoidCallback? onAddNode,
  VoidCallback? onPaste,
  bool canPaste = true,
  VoidCallback? onSelectAll,
  VoidCallback? onFrameSelected,
  bool canFrameSelected = true,
  VoidCallback? onFrameAll,
  VoidCallback? onAddNote,
  VoidCallback? onGroup,
  bool canGroup = true,
  VoidCallback? onCollapseAll,
  bool canCollapseAll = true,
  VoidCallback? onExpandAll,
  bool canExpandAll = true,
}) =>
    _show(context, at, [
      [
        _Row('add-node', l10n.graphAddNode, onAddNode, action: 'console.open'),
        _Row('paste', l10n.menuPaste, onPaste,
            action: 'edit.paste', enabled: canPaste),
      ],
      [
        _Row('select-all', l10n.menuSelectAll, onSelectAll,
            action: 'edit.select.all'),
        _Row('frame-selected', l10n.graphFrameSelected, onFrameSelected,
            action: 'nodes.frame.selected', enabled: canFrameSelected),
        _Row('frame-all', l10n.graphFrameAll, onFrameAll,
            action: 'nodes.frame.all'),
      ],
      [
        _Row('add-note', l10n.tipGraphNote, onAddNote),
        _Row('group', l10n.graphGroup, onGroup,
            action: 'layer.group', enabled: canGroup),
      ],
      [
        _Row('collapse-all', l10n.collapseAll, onCollapseAll,
            enabled: canCollapseAll),
        _Row('expand-all', l10n.expandAll, onExpandAll,
            enabled: canExpandAll),
      ],
    ]);

/// A canvas toolbar's menu: its switches as ticked rows, then the framing.
/// A switch the canvas doesn't have is left out.
Future<void> showGraphToolbarMenu(
  BuildContext context,
  Offset at, {
  bool? autoWire,
  VoidCallback? onAutoWire,
  bool? heal,
  VoidCallback? onHeal,
  required bool snap,
  required VoidCallback onSnap,
  required VoidCallback onFrameSelected,
  bool canFrameSelected = true,
  required VoidCallback onFrameAll,
}) =>
    _show(context, at, [
      [
        _Row('auto-wire', l10n.graphAutoWire, onAutoWire, ticked: autoWire),
        _Row('heal', l10n.graphHeal, onHeal, ticked: heal),
        _Row('snap', l10n.menuSnapToGrid, onSnap, ticked: snap),
      ],
      [
        _Row('frame-selected', l10n.graphFrameSelected, onFrameSelected,
            action: 'nodes.frame.selected', enabled: canFrameSelected),
        _Row('frame-all', l10n.graphFrameAll, onFrameAll,
            action: 'nodes.frame.all'),
      ],
    ]);
