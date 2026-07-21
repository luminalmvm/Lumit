// The live Project panel (phase F1): one row per document item, folders
// nesting their children. Selection and drag are later slices — a row shows
// its type icon, its name, and lights on hover. An empty document shows a
// quiet hint.

import 'package:flutter/widgets.dart';

import '../bridge/bridge.dart';
import '../icons/icons.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

class ProjectPanel extends StatelessWidget {
  final AppStateStub app;
  const ProjectPanel({super.key, required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: app,
      builder: (context, _) {
        final snapshot = app.snapshot;
        final items = snapshot?.items ?? const <BridgeItem>[];
        if (items.isEmpty) {
          return Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 240),
              child: Text(
                'No items yet — import footage or create a composition',
                style: t.small,
                textAlign: TextAlign.center,
              ),
            ),
          );
        }
        final rows = <Widget>[];
        void walk(BridgeItem item, int depth) {
          rows.add(_ProjectRow(item: item, depth: depth));
          for (final child in item.children) {
            walk(child, depth + 1);
          }
        }

        for (final item in items) {
          walk(item, 0);
        }
        return ListView(
          padding: const EdgeInsets.symmetric(vertical: 4),
          children: rows,
        );
      },
    );
  }
}

/// One Project panel row: a type icon (tinted with the layer colours where it
/// reads well), the item name, indented 14 px per level. Hover fills with
/// `surface4`.
class _ProjectRow extends StatefulWidget {
  final BridgeItem item;
  final int depth;
  const _ProjectRow({required this.item, required this.depth});

  @override
  State<_ProjectRow> createState() => _ProjectRowState();
}

class _ProjectRowState extends State<_ProjectRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final (icon, tint) = _iconFor(widget.item.kind, t);
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Container(
        height: 22,
        color: _hover ? t.surface4 : null,
        padding: EdgeInsets.only(left: 6.0 + widget.depth * 14.0, right: 6),
        child: Row(
          children: [
            lumitIcon(icon, size: 14, color: tint),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                widget.item.name,
                style: t.body,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// The icon and its tint for a kind. Footage/composition/solid take their
  /// layer colours; folders take the muted text colour (they are structure, not
  /// content); an unknown kind falls back to a plain muted dot-style icon.
  (LumitIcon, Color) _iconFor(BridgeItemKind kind, LumitTheme t) =>
      switch (kind) {
        BridgeItemKind.footage => (LumitIcon.footage, t.layer.footage),
        BridgeItemKind.folder => (LumitIcon.folder, t.textMuted),
        BridgeItemKind.composition => (LumitIcon.comp, t.layer.precomp),
        BridgeItemKind.solid => (LumitIcon.solid, t.layer.solid),
        BridgeItemKind.unknown => (LumitIcon.footage, t.textMuted),
      };
}
