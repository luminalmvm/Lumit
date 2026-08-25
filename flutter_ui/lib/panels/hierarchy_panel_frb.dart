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

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'placeholder.dart';
import 'timeline_extras_frb.dart';

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

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context);
    final comp = ui.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.nodes,
        title: l10n.panelHierarchy,
        hint: l10n.selectACompositionFirst,
      );
    }

    // The fronted comp's rows come from the read model — zero bridge calls
    // (K-184); an expanded precomp's inside is read as it opens.
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) {
        final rows = <Widget>[];
        _walkEntries(context, ui.model.layers, ui, rows, 0);

        if (rows.isEmpty) {
          return Center(
            child: Text(l10n.hierarchyEmpty, style: t.small),
          );
        }
        return ListView(
          padding: const EdgeInsets.symmetric(vertical: 4),
          children: rows,
        );
      },
    );
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

      rows.add(_HierarchyRow(
        key: ValueKey<String>('hierarchy-row-$id'),
        name: info.name,
        kind: kind,
        depth: depth,
        // The whole selection, not just the primary (K-217): a layer chosen in
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
            : () => setState(() => ui.setSelectedComp(nested)),
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
  final int depth;
  final bool selected;
  final bool expandable;
  final bool open;
  final VoidCallback onTap;
  final VoidCallback? onToggle;
  final VoidCallback? onOpenComp;

  const _HierarchyRow({
    super.key,
    required this.name,
    required this.kind,
    required this.depth,
    required this.selected,
    required this.expandable,
    required this.open,
    required this.onTap,
    this.onToggle,
    this.onOpenComp,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      onDoubleTap: onOpenComp,
      child: Container(
        height: 22,
        color: selected ? t.surface2 : null,
        padding: EdgeInsets.only(left: 6 + depth * _indent, right: 6),
        child: Row(
          children: [
            SizedBox(
              width: 14,
              child: expandable
                  ? GestureDetector(
                      behavior: HitTestBehavior.opaque,
                      onTap: onToggle,
                      child: lumitIcon(
                        open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                        size: iconSize,
                        color: t.textMuted,
                      ),
                    )
                  : null,
            ),
            lumitIcon(iconForKind(kind), size: iconSize, color: t.textMuted),
            const SizedBox(width: 6),
            Expanded(
              child: Text(name, style: t.body, overflow: TextOverflow.ellipsis),
            ),
            if (expandable)
              Text('precomp', style: t.small.copyWith(color: t.textMuted)),
          ],
        ),
      ),
    );
  }
}
