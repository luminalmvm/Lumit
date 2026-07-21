// Renders the dock tree (state/dock.dart): weighted splits with draggable
// dividers, tab groups as pill tab bars (dock.rs::tab_ui styling), solo panes
// bare (K-086), and the Sharp/Round pane chrome (K-092). Tab drag-to-redock
// and pop-out windows are checklist items, not yet built.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../state/dock.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

typedef PanelBuilder = Widget Function(BuildContext context, Panel panel);

class DockWidget extends StatefulWidget {
  final DockSplit root;
  final PanelBuilder buildPanel;
  final VoidCallback onLayoutChanged;

  /// The panel that last took a click — it wears the accent boundary so the
  /// keyboard's home is always visible (Shell::active_panel).
  final ValueNotifier<Panel?> activePanel;

  /// Called when a pane's context menu asks to pop out into its own window.
  final void Function(Panel) onPopOut;

  const DockWidget({
    super.key,
    required this.root,
    required this.buildPanel,
    required this.onLayoutChanged,
    required this.activePanel,
    required this.onPopOut,
  });

  @override
  State<DockWidget> createState() => _DockWidgetState();
}

class _DockWidgetState extends State<DockWidget> {
  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      color: t.surface0,
      padding: EdgeInsets.all(t.tokens.windowInset),
      child: _buildNode(context, widget.root),
    );
  }

  Widget _buildNode(BuildContext context, DockNode node) => switch (node) {
        DockPane(:final panel) => _PaneChrome(
            bare: true,
            panel: panel,
            activePanel: widget.activePanel,
            onPopOut: widget.onPopOut,
            child: widget.buildPanel(context, panel),
          ),
        DockTabs() => _TabGroup(
            tabs: node,
            buildPanel: widget.buildPanel,
            activePanel: widget.activePanel,
            onPopOut: widget.onPopOut,
            onChanged: () {
              setState(() {});
              widget.onLayoutChanged();
            },
          ),
        DockSplit() => _buildSplit(context, node),
      };

  Widget _buildSplit(BuildContext context, DockSplit split) {
    final t = ThemeScope.of(context).theme;
    final horizontal = split.axis == DockAxis.horizontal;
    final children = <Widget>[];
    for (var i = 0; i < split.children.length; i++) {
      children.add(Expanded(
        // Flex is integer; scale the share up to keep precision.
        flex: (split.shares[i] * 10000).round().clamp(1, 1 << 30),
        child: _buildNode(context, split.children[i]),
      ));
      if (i < split.children.length - 1) {
        children.add(_Divider(
          horizontal: horizontal,
          gap: t.tokens.tileGap,
          onDrag: (delta, totalExtent) {
            setState(() {
              _resize(split, i, horizontal ? delta.dx : delta.dy, totalExtent);
            });
            widget.onLayoutChanged();
          },
        ));
      }
    }
    return horizontal ? Row(children: children) : Column(children: children);
  }

  /// Move the boundary between child i and i+1 by `deltaPx` of `totalExtent`.
  void _resize(DockSplit split, int i, double deltaPx, double totalExtent) {
    if (totalExtent <= 0) return;
    final total = split.shares.reduce((a, b) => a + b);
    final deltaShare = deltaPx / totalExtent * total;
    const minShare = 0.05;
    final a = split.shares[i] + deltaShare;
    final b = split.shares[i + 1] - deltaShare;
    if (a < minShare || b < minShare) return;
    split.shares[i] = a;
    split.shares[i + 1] = b;
  }
}

class _Divider extends StatefulWidget {
  final bool horizontal;
  final double gap;
  final void Function(Offset delta, double totalExtent) onDrag;

  const _Divider({
    required this.horizontal,
    required this.gap,
    required this.onDrag,
  });

  @override
  State<_Divider> createState() => _DividerState();
}

class _DividerState extends State<_Divider> {
  bool _hover = false;
  bool _dragging = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Sharp: hairline-toned gap, brighter on hover/drag. Round: canvas-toned
    // gap, hairline on hover, accent while dragging (dock.rs::resize_stroke).
    final sharp = t.shape == ThemeShape.sharp;
    final idle = sharp ? t.surface2 : t.surface0;
    final colour = _dragging
        ? (sharp ? t.textPrimary : t.accent)
        : _hover
            ? (sharp ? t.textPrimary : t.hairlineStrong)
            : idle;
    // The visible gap keeps the token width; the hit area is padded to a
    // comfortable 7 px so a 1 px hairline is still grabbable.
    final hit = widget.gap < 7.0 ? 7.0 : widget.gap;
    return MouseRegion(
      cursor: widget.horizontal
          ? SystemMouseCursors.resizeColumn
          : SystemMouseCursors.resizeRow,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onPanStart: (_) => setState(() => _dragging = true),
        onPanEnd: (_) => setState(() => _dragging = false),
        onPanCancel: () => setState(() => _dragging = false),
        onPanUpdate: (d) {
          final parent = context
              .findAncestorRenderObjectOfType<RenderBox>()
              ?.size;
          final extent = parent == null
              ? 0.0
              : (widget.horizontal ? parent.width : parent.height);
          widget.onDrag(d.delta, extent);
        },
        child: SizedBox(
          width: widget.horizontal ? hit : null,
          height: widget.horizontal ? null : hit,
          child: Center(
            child: Container(
              width: widget.horizontal ? widget.gap : null,
              height: widget.horizontal ? null : widget.gap,
              color: colour,
            ),
          ),
        ),
      ),
    );
  }
}

/// A tab group: the 26 px tab bar of pill tabs plus the active pane's body.
class _TabGroup extends StatelessWidget {
  final DockTabs tabs;
  final PanelBuilder buildPanel;
  final VoidCallback onChanged;
  final ValueNotifier<Panel?> activePanel;
  final void Function(Panel) onPopOut;

  const _TabGroup({
    required this.tabs,
    required this.buildPanel,
    required this.onChanged,
    required this.activePanel,
    required this.onPopOut,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final barColour =
        t.shape == ThemeShape.sharp ? t.surface2 : t.surface0;
    return Column(
      children: [
        Container(
          height: 26,
          color: barColour,
          child: Row(
            children: [
              // The pill strip scrolls when the group is narrower than its
              // tabs, as egui_tiles' tab bar does.
              Expanded(
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: [
                      for (var i = 0; i < tabs.children.length; i++)
                        _TabPill(
                          title: tabs.children[i].panel.title,
                          active: i == tabs.active,
                          onPressed: () {
                            tabs.active = i;
                            onChanged();
                          },
                        ),
                    ],
                  ),
                ),
              ),
              // The pop-out button for the active tab (top_bar_right_ui).
              LumitTooltip(
                message: 'Pop out into its own window',
                child: HouseButton(
                  frameless: true,
                  small: true,
                  onPressed: () => onPopOut(tabs.activePane.panel),
                  child: lumitIcon(LumitIcon.popOut,
                      size: 12, color: t.textMuted),
                ),
              ),
              const SizedBox(width: 4),
            ],
          ),
        ),
        Expanded(
          child: _PaneChrome(
            bare: false,
            panel: tabs.activePane.panel,
            activePanel: activePanel,
            onPopOut: onPopOut,
            child: buildPanel(context, tabs.activePane.panel),
          ),
        ),
      ],
    );
  }
}

class _TabPill extends StatefulWidget {
  final String title;
  final bool active;
  final VoidCallback onPressed;

  const _TabPill({
    required this.title,
    required this.active,
    required this.onPressed,
  });

  @override
  State<_TabPill> createState() => _TabPillState();
}

class _TabPillState extends State<_TabPill> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final Color fill;
    final Color textColour;
    Border? border;
    if (widget.active) {
      fill = t.surface1;
      textColour = t.textPrimary;
      border = Border.all(color: t.accent, width: 1);
    } else if (_hover) {
      fill = t.surface3;
      textColour = t.textPrimary;
      border = Border.all(color: t.hairlineStrong, width: 1);
    } else {
      fill = t.surface2;
      textColour = t.textMuted;
    }
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        onTap: widget.onPressed,
        child: Container(
          margin: const EdgeInsets.symmetric(horizontal: 2, vertical: 4),
          padding: const EdgeInsets.symmetric(horizontal: 10),
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: fill,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: border,
          ),
          child: Text(widget.title, style: t.body.copyWith(color: textColour)),
        ),
      ),
    );
  }
}

/// The pane body chrome: Sharp draws edge-to-edge on `surface1`; Round wraps
/// the content in a rounded, shadowed, padded card (dock.rs::pane_ui). Any
/// click inside makes this the active panel, which wears the accent boundary
/// (Shell::active_panel); a right-click on a bare pane offers "pop out"
/// (bare_pane_ui — tabbed panes get it from the tab bar's own button).
class _PaneChrome extends StatelessWidget {
  final bool bare;
  final Panel panel;
  final ValueNotifier<Panel?> activePanel;
  final void Function(Panel) onPopOut;
  final Widget child;

  const _PaneChrome({
    required this.bare,
    required this.panel,
    required this.activePanel,
    required this.onPopOut,
    required this.child,
  });

  void _contextMenu(BuildContext context, Offset globalPos) {
    showLumitPopup<void>(
      context: context,
      position: globalPos,
      builder: (close) => FloatSurface(
        child: MenuRow(
          onPressed: () {
            close(null);
            onPopOut(panel);
          },
          child: const Text('Pop out into its own window'),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final round = t.shape == ThemeShape.round;
    return ValueListenableBuilder<Panel?>(
      valueListenable: activePanel,
      builder: (context, active, _) => Listener(
        // Any press claims focus for this panel, before the content handles
        // the event (the egui edge follows the last click the same way).
        onPointerDown: (_) => activePanel.value = panel,
        child: GestureDetector(
          behavior: HitTestBehavior.translucent,
          onSecondaryTapDown:
              bare ? (d) => _contextMenu(context, d.globalPosition) : null,
          child: Container(
            decoration: BoxDecoration(
              color: t.surface1,
              borderRadius:
                  round ? BorderRadius.circular(t.tokens.cardRadius) : null,
              boxShadow: round ? t.tokens.cardShadow : null,
            ),
            // The accent boundary paints over the content's edge, like the
            // egui overlay stroke at Order::Middle.
            foregroundDecoration: active == panel
                ? BoxDecoration(
                    border: Border.all(color: t.accent, width: 1),
                    borderRadius: round
                        ? BorderRadius.circular(t.tokens.cardRadius)
                        : null,
                  )
                : null,
            padding: round ? EdgeInsets.all(t.tokens.cardPadding) : null,
            clipBehavior: round ? Clip.antiAlias : Clip.none,
            child: child,
          ),
        ),
      ),
    );
  }
}
