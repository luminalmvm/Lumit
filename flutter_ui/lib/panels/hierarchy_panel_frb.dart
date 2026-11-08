// The Hierarchy panel, on the flutter_rust_bridge API.
//
// The front composition's layers as a tree, with precomp layers expandable to
// show what is inside them. Clicking a row selects that layer; clicking into a
// precomp fronts it, which is how you get from "this shot uses that comp" to
// editing that comp.
//
// **The recursion has a depth limit, and it is not paranoia.** A precomp cycle
// is an invalid document state the engine guards at insertion, but a panel that
// walks the tree has to survive one anyway: a document written by a newer Lumit,
// or a file edited by hand, must open rather than hang the interface. Ten levels
// is far past any real nesting and cheap to enforce.

import 'package:flutter/gestures.dart' show kPrimaryButton;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart';
import 'timeline_layer_menu_frb.dart' show revealInProject, showMenuEntriesAt;
import 'timeline_outline_row_frb.dart' show showLayerRowMenu;

/// How far the tree will walk before it stops, whatever the document says.
const int _maxDepth = 10;

/// How far each level indents.
const double _indent = 14;

class HierarchyPanelFrb extends StatefulWidget {
  const HierarchyPanelFrb({super.key});

  @override
  State<HierarchyPanelFrb> createState() => _HierarchyPanelFrbState();
}

class _HierarchyPanelFrbState extends State<HierarchyPanelFrb> {
  /// Which precomp rows are open, by layer id.
  final Set<String> _open = {};

  /// Every precomp row drawn, by layer id, for Expand all.
  final Set<String> _openable = {};

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    // Nothing to show is nothing shown: no comp, or a comp with no layers.
    if (ui.selectedComp == null) return const SizedBox.expand();

    // The fronted comp's rows come from the read model — zero bridge calls;
    // an expanded precomp's inside is read as it opens.
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) {
        final rows = <Widget>[];
        _openable.clear();
        _walkEntries(context, ui.model.layers, ui, rows, 0);
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapDown: (d) =>
              _menu(context, d.globalPosition, ui, null, null),
          child: ListView(
            padding: const EdgeInsets.symmetric(vertical: 4),
            children: rows,
          ),
        );
      },
    );
  }

  /// The menu on a row, or with no [layer] on the ground under the rows. A
  /// layer of the comp in front has the one layer menu it has everywhere, with
  /// this panel's own rows in it, and a layer inside a Precomp has what can be
  /// done from here. A row opens or closes itself; the ground does them all.
  void _menu(BuildContext context, Offset at, LumitUiState ui,
      LayerReference? layer, CompositionReference? nested) {
    final id = layer?.internallayerId.toString();
    final view = [
      if (nested != null)
        MenuEntry(
            _open.contains(id) ? l10n.collapse : l10n.expand,
            () => setState(() {
                  if (!_open.remove(id)) _open.add(id!);
                })),
      if (layer == null && _openable.isNotEmpty) ...[
        MenuEntry(l10n.expandAll, () => setState(() => _open.addAll(_openable))),
        MenuEntry(l10n.collapseAll,
            _open.isEmpty ? null : () => setState(_open.clear)),
      ],
    ];
    if (layer != null &&
        ui.model.layers
            .any((e) => e.layer.internallayerId == layer.internallayerId)) {
      showLayerRowMenu(context, layer.internallayerId, at, view: view);
      return;
    }
    if (layer == null && view.isEmpty) return;
    // Read here: a menu is a gesture. A text layer or a camera has no item.
    final source = layer?.getSourceItem();
    showMenuEntriesAt(context, at, [
      if (layer != null && nested != null)
        MenuEntry(l10n.projectOpen, () => ui.openNestedComp(layer, nested)),
      ...view,
      if (layer != null) ...[
        if (view.isNotEmpty) MenuEntry.divider(),
        MenuEntry(l10n.menuRevealInProject,
            source == null ? null : () => revealInProject(ui, source)),
      ],
    ]);
  }

  void _walkEntries(
    BuildContext context,
    List<BridgeLayerEntry> entries,
    LumitUiState ui,
    List<Widget> rows,
    int depth,
  ) {
    if (depth >= _maxDepth) return;

    for (final entry in entries) {
      final layer = entry.layer;
      final id = layer.internallayerId.toString();
      final info = entry.info;
      final kind = info.kind;
      final nested = kind == BridgeLayerKind.precomp ? _compOf(layer) : null;
      final open = _open.contains(id);
      if (nested != null) _openable.add(id);

      rows.add(_HierarchyRow(
        key: ValueKey<String>('hierarchy-row-$id'),
        name: info.name,
        kind: kind,
        label: info.label,
        depth: depth,
        // The whole selection, not just the primary: a layer chosen in
        // the Timeline with Ctrl held reads as chosen here too, or the two
        // panels would be showing two different answers to one question.
        selected: ui.selectedLayerIds.contains(layer.internallayerId),
        expandable: nested != null,
        open: open,
        onTap: () => setState(() {
          final keys = HardwareKeyboard.instance;
          if (keys.isControlPressed || keys.isMetaPressed) {
            ui.toggleSelected(layer);
          } else {
            ui.setSelection([layer]);
          }
        }),
        onToggle: nested == null
            ? null
            : () => setState(() {
                  if (open) {
                    _open.remove(id);
                  } else {
                    _open.add(id);
                  }
                }),
        onOpenComp: nested == null
            ? null
            : () => setState(() => ui.openNestedComp(layer, nested)),
        onMenu: (at) => _menu(context, at, ui, layer, nested),
      ));

      if (nested != null && open) {
        // Inside a precomp: not the fronted comp, so not in the model — one
        // getModel read per expanded precomp per rebuild is the honest cost.
        _walkEntries(context, nested.getModel().layers, ui, rows, depth + 1);
      }
    }
  }

  /// The composition a precomp layer draws, if it is still in the document.
  CompositionReference? _compOf(LayerReference layer) {
    final item = layer.getSourceItem();
    return item is ItemReference_Composition ? item.field0 : null;
  }
}

class _HierarchyRow extends StatelessWidget {
  final String name;
  final BridgeLayerKind kind;
  final int label;
  final int depth;
  final bool selected;
  final bool expandable;
  final bool open;
  final VoidCallback onTap;
  final VoidCallback? onToggle;
  final VoidCallback? onOpenComp;
  final ValueChanged<Offset> onMenu;

  const _HierarchyRow({
    super.key,
    required this.name,
    required this.kind,
    required this.label,
    required this.depth,
    required this.selected,
    required this.expandable,
    required this.open,
    required this.onTap,
    required this.onMenu,
    this.onToggle,
    this.onOpenComp,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return HoverFill(
      faint: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onSecondaryTapDown: (d) {
          if (!selected) onTap();
          onMenu(d.globalPosition);
        },
        child: Container(
          height: 22,
          color: selected ? t.surface2 : null,
          child: Stack(
            children: [
              // A line down from each level above, joining a precomp to what
              // is inside it.
              for (var level = 0; level < depth; level++)
                PositionedDirectional(
                  start: 6 + level * _indent + 6,
                  top: 0,
                  bottom: 0,
                  child: Container(width: 1, color: t.hairline),
                ),
              Padding(
                padding: EdgeInsetsDirectional.only(
                    start: 6 + depth * _indent, end: 6),
                child: Row(
                  children: [
                    SizedBox(
                      width: _indent,
                      child: expandable
                          ? HoverMark(
                              builder: (context, lit) => GestureDetector(
                                key: ValueKey<String>('hierarchy-twirl-$name'),
                                behavior: HitTestBehavior.opaque,
                                onTap: onToggle,
                                child: TwirlTurn(
                                  open: open,
                                  child: lumitIcon(
                                    open
                                        ? LumitIcon.twirlOpen
                                        : LumitIcon.twirlClosed,
                                    size: iconSize,
                                    color: Color.lerp(
                                        t.textMuted, t.textPrimary, lit)!,
                                  ),
                                ),
                              ),
                            )
                          : null,
                    ),
                    // On the down stroke beside the twirl, not on it: waiting
                    // out a double-click on a precomp read as the panel lagging.
                    Expanded(
                      child: Listener(
                        onPointerDown: (e) {
                          if (e.buttons == kPrimaryButton) onTap();
                        },
                        child: GestureDetector(
                          behavior: HitTestBehavior.opaque,
                          onDoubleTap: onOpenComp,
                          child: Row(
                            children: [
                              lumitIcon(iconForKind(kind),
                                  size: iconSize,
                                  color: label != 0
                                      ? t.labelColour(label)
                                      : t.textMuted),
                              const SizedBox(width: 6),
                              Expanded(
                                child: Text(name,
                                    style: t.body.copyWith(
                                        color: selected
                                            ? t.textPrimary
                                            : t.textSecondary),
                                    overflow: TextOverflow.ellipsis),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
