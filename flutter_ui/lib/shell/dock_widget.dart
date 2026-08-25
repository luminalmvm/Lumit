// Renders the dock tree (state/dock.dart): weighted splits with draggable
// dividers, tab groups as pill tab bars (dock.rs::tab_ui styling), solo panes
// bare (K-086), and the Sharp/Round pane chrome (K-092). A tab drags to re-dock
// (dock.rs drag-to-redock, via egui_tiles): a ghost pill follows the cursor, the
// hovered pane shows a drop-zone preview, and release commits the move through
// movePanel. Every pane is a drop target, bare ones included.

import 'package:flutter/rendering.dart' show RenderOffstage;
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/dock.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

typedef PanelBuilder = Widget Function(BuildContext context, Panel panel);

/// A pointer must travel this far before a press on a tab becomes a re-dock
/// drag rather than a click.
const double _dragSlop = 6.0;

/// **The width below which a panel stops giving way and starts sliding**
/// (K-451's degradation ladder, step 5, and the "every panel declares a
/// minimum" the same entry asks for).
///
/// In plain terms: a panel shrinks by shedding — a word here, a column there —
/// but every panel reaches a width where there is nothing left to shed and the
/// only honest answers are to slide the content sideways or to paint outside
/// the box. This number is where that line is, and [PanelFloor] is what does
/// the sliding. Dragging a seam past it is refused outright, so the ordinary
/// way of making a panel narrow never reaches the scrolling case at all; the
/// scroll is for the window that is simply too small to hold the workspace.
///
/// The numbers are what each panel measurably needs once its own ladder has
/// run out — the width its narrowest useful arrangement lays out at without
/// overflowing — and `panel_width_sweep_test` is what keeps them honest.
double panelMinWidth(Panel panel) => switch (panel) {
      // A row is a twirl, a type mark, a name, its state badges and the
      // metadata columns still standing at this width; that is what 300 buys.
      Panel.project => 300,
      // The picture itself will draw at any size, and both strips slide on
      // their own below the width they want, so the Viewer's floor is only
      // about there being a picture worth looking at.
      Panel.viewer => 200,
      // The outline's own columns, before a single lane of time.
      Panel.timeline => 220,
      // A parameter row: its name, its wells and its keyframe marks.
      Panel.effectControls => 300,
      Panel.graph => 320,
      Panel.effectsAndPresets => 180,
      Panel.node => 200,
      Panel.scopes => 160,
      Panel.hierarchy => 180,
      Panel.easing => 220,
      Panel.debug => 180,
    };

/// The smallest width a whole dock subtree can be drawn at: a pane is its
/// panel's floor, a tab group is the widest of the panels stacked in it, and a
/// split is the sum of its children across, or the widest of them down.
double dockMinWidth(DockNode node) => switch (node) {
      DockPane(:final panel) => panelMinWidth(panel),
      DockTabs(:final children) => children.isEmpty
          ? 0
          : children
              .map((c) => panelMinWidth(c.panel))
              .reduce((a, b) => a > b ? a : b),
      DockSplit(:final axis, :final children) => children.isEmpty
          ? 0
          : axis == DockAxis.horizontal
              ? children.map(dockMinWidth).reduce((a, b) => a + b)
              : children.map(dockMinWidth).reduce((a, b) => a > b ? a : b),
    };

/// Holds [child] to [minWidth] and slides it sideways when the pane is
/// narrower than that (§12A.6's ladder, step 5).
///
/// In plain terms: rather than squeeze a panel into a box it cannot fit, give
/// it the box it needs and let the user push it left and right. Every docked
/// panel is wrapped in one of these, which is why a panel nobody has audited
/// for narrow widths still cannot paint outside its pane.
class PanelFloor extends StatelessWidget {
  final double minWidth;
  final Widget child;

  const PanelFloor({super.key, required this.minWidth, required this.child});

  @override
  Widget build(BuildContext context) => LayoutBuilder(
        builder: (context, box) {
          // Nothing to hold it to, or room to spare: the panel lays out
          // exactly as it always did, and this wrapper costs one LayoutBuilder.
          if (!box.hasBoundedWidth ||
              !box.hasBoundedHeight ||
              box.maxWidth >= minWidth) {
            return child;
          }
          return SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: SizedBox(
              width: minWidth,
              // The height has to be handed over as a number: a panel is a
              // Column with an Expanded in it, and inside a scroll view the
              // cross axis arrives loose.
              height: box.maxHeight,
              child: child,
            ),
          );
        },
      );
}

class DockWidget extends StatefulWidget {
  final DockSplit root;
  final PanelBuilder buildPanel;
  final VoidCallback onLayoutChanged;

  /// The panel that last took a click — it wears the accent boundary so the
  /// keyboard's home is always visible (Shell::active_panel).
  final ValueNotifier<Panel?> activePanel;

  const DockWidget({
    super.key,
    required this.root,
    required this.buildPanel,
    required this.onLayoutChanged,
    required this.activePanel,
  });

  @override
  State<DockWidget> createState() => _DockWidgetState();
}

class _DockWidgetState extends State<DockWidget> {
  // One stable key per panel, used to hit-test the pane rects during a drag.
  // A panel keeps its key even while it is an inactive tab (unbuilt, so its
  // key resolves to no context and is skipped).
  late final Map<Panel, GlobalKey> _paneKeys = {
    for (final p in Panel.values) p: GlobalKey(),
  };
  late final _DragController _drag = _DragController(
    paneKeys: _paneKeys,
    onGhostShow: _showGhost,
    onGhostHide: _removeGhost,
    onCommit: _commitMove,
  );
  OverlayEntry? _ghost;

  @override
  void dispose() {
    _removeGhost();
    _drag.dispose();
    super.dispose();
  }

  void _showGhost() {
    _removeGhost();
    _ghost = OverlayEntry(builder: (_) => _GhostLayer(drag: _drag));
    Overlay.of(context).insert(_ghost!);
  }

  void _removeGhost() {
    _ghost?.remove();
    _ghost = null;
  }

  void _commitMove(Panel dragged, Panel target, DropPosition pos) {
    setState(() => movePanel(widget.root, dragged, target, pos));
    widget.onLayoutChanged();
  }

  /// Drop a panel out of the arrangement — the same thing the Window menu's
  /// tick does, reached from the tab's own right-click menu (K-521).
  void _closePanel(Panel panel) {
    setState(() => setPanelVisible(widget.root, panel, false));
    widget.onLayoutChanged();
  }

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
            panel: panel,
            activePanel: widget.activePanel,
            drag: _drag,
            child: widget.buildPanel(context, panel),
          ),
        DockTabs() => _TabGroup(
            tabs: node,
            buildPanel: widget.buildPanel,
            activePanel: widget.activePanel,
            drag: _drag,
            onClose: _closePanel,
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
    // **The seam stops at the panels' own minimums** (K-451: "every panel
    // declares a minimum; the dock will not shrink it past that"). Across a
    // horizontal split that minimum is a width, so the drag is refused rather
    // than clamped — a seam that slid on while the panel underneath stopped
    // moving was how a panel ended up narrower than anything it could draw.
    if (split.axis == DockAxis.horizontal) {
      final aPx = a / total * totalExtent;
      final bPx = b / total * totalExtent;
      if (aPx < dockMinWidth(split.children[i]) ||
          bPx < dockMinWidth(split.children[i + 1])) {
        return;
      }
    }
    split.shares[i] = a;
    split.shares[i + 1] = b;
  }
}

/// The live state of a re-dock drag, shared between the dragged source (a tab
/// pill), the ghost pill and every pane's drop preview.
/// It resolves the hovered pane and drop position by hit-testing the pointer
/// against the pane rects each update, because MouseRegion does not fire while
/// a pointer is captured by a drag.
class _DragController extends ChangeNotifier {
  final Map<Panel, GlobalKey> paneKeys;
  final VoidCallback onGhostShow;
  final VoidCallback onGhostHide;
  final void Function(Panel dragged, Panel target, DropPosition pos) onCommit;

  _DragController({
    required this.paneKeys,
    required this.onGhostShow,
    required this.onGhostHide,
    required this.onCommit,
  });

  Panel? dragged;
  LumitTheme? theme;
  Offset pointer = Offset.zero;
  Panel? hoveredPanel;
  DropPosition? dropPosition;

  void start(Panel panel, Offset globalPos, LumitTheme t) {
    dragged = panel;
    theme = t;
    pointer = globalPos;
    _resolve();
    onGhostShow();
    notifyListeners();
  }

  void update(Offset globalPos) {
    pointer = globalPos;
    _resolve();
    notifyListeners();
  }

  void finish() {
    final dragged = this.dragged;
    final target = hoveredPanel;
    final pos = dropPosition;
    _reset();
    onGhostHide();
    notifyListeners();
    if (dragged != null && target != null && pos != null) {
      onCommit(dragged, target, pos);
    }
  }

  void cancel() {
    _reset();
    onGhostHide();
    notifyListeners();
  }

  void _reset() {
    dragged = null;
    theme = null;
    hoveredPanel = null;
    dropPosition = null;
  }

  /// Find the pane under the pointer and the drop position within it.
  void _resolve() {
    hoveredPanel = null;
    dropPosition = null;
    for (final entry in paneKeys.entries) {
      final ctx = entry.value.currentContext;
      if (ctx == null) continue;
      final box = ctx.findRenderObject() as RenderBox?;
      if (box == null || !box.attached) continue;
      // A hidden tab is built (to keep its panel state) but sits under an
      // offstage ancestor. It still resolves a render object and would report
      // a rect overlapping the visible pane, so skip it — only the on-stage
      // pane is a valid drop target.
      if (!_onStage(box)) continue;
      final rect = box.localToGlobal(Offset.zero) & box.size;
      if (rect.contains(pointer)) {
        hoveredPanel = entry.key;
        dropPosition = _positionIn(rect, pointer);
        return;
      }
    }
  }

  /// Whether [box] has no offstage ancestor — i.e. it is actually painted.
  static bool _onStage(RenderObject box) {
    RenderObject? node = box;
    while (node != null) {
      if (node is RenderOffstage && node.offstage) return false;
      final parent = node.parent;
      node = parent is RenderObject ? parent : null;
    }
    return true;
  }

  /// The inner ~50% of both axes is a stack; outside it, the nearest edge
  /// picks a side to split off.
  static DropPosition _positionIn(Rect rect, Offset p) {
    final fx = ((p.dx - rect.left) / rect.width).clamp(0.0, 1.0);
    final fy = ((p.dy - rect.top) / rect.height).clamp(0.0, 1.0);
    if ((fx - 0.5).abs() < 0.25 && (fy - 0.5).abs() < 0.25) {
      return DropPosition.stack;
    }
    final left = fx, right = 1 - fx, top = fy, bottom = 1 - fy;
    final nearest = [left, right, top, bottom].reduce((a, b) => a < b ? a : b);
    if (nearest == left) return DropPosition.left;
    if (nearest == right) return DropPosition.right;
    if (nearest == top) return DropPosition.above;
    return DropPosition.below;
  }
}

/// Turns a press-and-drag on its child into a re-dock drag: a press that
/// travels past the slop starts the drag and drives the controller, leaving a
/// plain tap (a tab click) to the child's own gesture handling. Uses a raw
/// Listener so it stays out of the gesture arena and never fights the tab
/// strip's horizontal scroll.
class _DragSource extends StatefulWidget {
  final Panel panel;
  final _DragController drag;
  final Widget child;

  const _DragSource({
    required this.panel,
    required this.drag,
    required this.child,
  });

  @override
  State<_DragSource> createState() => _DragSourceState();
}

class _DragSourceState extends State<_DragSource> {
  Offset? _downAt;
  bool _dragging = false;

  /// Theme snapshot taken at pointer-down, when the element is certainly
  /// live. A press elsewhere can rebuild the dock (the active-panel edge)
  /// and deactivate this element while the pointer stays captured, so the
  /// move handler must never look up inherited widgets through `context`.
  LumitTheme? _theme;

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerDown: (e) {
        _downAt = e.position;
        _dragging = false;
        _theme = ThemeScope.of(context).theme;
      },
      onPointerMove: (e) {
        final theme = _theme;
        if (_downAt == null || theme == null) return;
        if (!_dragging) {
          if ((e.position - _downAt!).distance < _dragSlop) return;
          _dragging = true;
          widget.drag.start(widget.panel, e.position, theme);
        } else {
          widget.drag.update(e.position);
        }
      },
      onPointerUp: (e) {
        if (_dragging) widget.drag.finish();
        _dragging = false;
        _downAt = null;
      },
      onPointerCancel: (e) {
        if (_dragging) widget.drag.cancel();
        _dragging = false;
        _downAt = null;
      },
      child: widget.child,
    );
  }
}

/// The ghost pill that follows the cursor during a drag, on the app Overlay.
class _GhostLayer extends StatelessWidget {
  final _DragController drag;
  const _GhostLayer({required this.drag});

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: drag,
        builder: (context, _) {
          final panel = drag.dragged;
          final t = drag.theme;
          if (panel == null || t == null) return const SizedBox.shrink();
          // The pointer is a window coordinate and the pill is placed in the
          // overlay's own space; at any UI scale but 100% those differ, and
          // the pill would trail the pointer by the difference (K-560).
          final at = overlayLocal(context, drag.pointer);
          return Positioned(
            left: at.dx + 10,
            top: at.dy + 8,
            child:
                IgnorePointer(child: _GhostPill(title: panel.title, theme: t)),
          );
        },
      );
}

/// The floating tab pill, styled like the active pill it was lifted from.
class _GhostPill extends StatelessWidget {
  final String title;
  final LumitTheme theme;
  const _GhostPill({required this.title, required this.theme});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
      decoration: BoxDecoration(
        color: theme.surface1,
        borderRadius: BorderRadius.circular(theme.tokens.controlRadius),
        border: Border.all(color: theme.accent, width: 1),
        boxShadow: theme.floatShadow,
      ),
      child: Text(title.toUpperCase(), style: theme.kickerOn),
    );
  }
}

/// The panel header's live-mark under Round (K-394, §12.1): a small accent dot
/// before the panel's name in its tab.
///
/// **Decorative and static.** It never blinks, never fills and never means
/// anything — it is not a status light, and no state may be routed through it.
/// Its diameter comes off the type scale (a third of the title's own size)
/// rather than a pixel count, so it stays a dot beside the word at every UI
/// scale instead of becoming a bead at one and a blob at another.
class _HeaderDot extends StatelessWidget {
  final TextStyle text;
  final Color colour;
  const _HeaderDot({required this.text, required this.colour});

  @override
  Widget build(BuildContext context) {
    final size = (text.fontSize ?? 11) / 3;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(color: colour, shape: BoxShape.circle),
    );
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
    // **The whole seam is painted, not just the hairline in it** (owner, desk
    // test). The hit padding is layout, so the panes stand `hit` apart whatever
    // the token says — and only `gap` of that was ever painted, leaving 3 px of
    // nothing either side of the hairline. Nothing composites onto whatever
    // happens to be underneath, and under Sharp that is the dock's `surface_0`
    // — the same value the Graph panel grounds its canvas in, so the seam
    // between the small Viewer and the Node panel read as a slot with the graph
    // showing through it. Every split is this one widget, so every seam had it.
    //
    // The ground goes edge to edge; the hairline stays [gap] wide inside it, so
    // hover and drag still light a hairline rather than the whole band.
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
          final parent =
              context.findAncestorRenderObjectOfType<RenderBox>()?.size;
          final extent = parent == null
              ? 0.0
              : (widget.horizontal ? parent.width : parent.height);
          widget.onDrag(d.delta, extent);
        },
        child: Container(
          width: widget.horizontal ? hit : null,
          height: widget.horizontal ? null : hit,
          color: idle,
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

/// The header strip every tab group wears: 22 px of `surface2` under Sharp
/// (docs/15-DESIGN.md §2.1 — "faint surfaces: tab bars, bottom bars, panel
/// headers"), with the pane body on `surface1` below it. Round keeps the canvas
/// showing between its cards instead, so its strip stays `surface0`.
const double _headerStripHeight = 22;

/// A tab group: the 22 px header strip of pill tabs plus the active pane's body.
class _TabGroup extends StatelessWidget {
  final DockTabs tabs;
  final PanelBuilder buildPanel;
  final VoidCallback onChanged;
  final ValueNotifier<Panel?> activePanel;
  final _DragController drag;

  /// Take a panel out of the arrangement, for the tab's right-click menu.
  final void Function(Panel) onClose;

  const _TabGroup({
    required this.tabs,
    required this.buildPanel,
    required this.onChanged,
    required this.activePanel,
    required this.drag,
    required this.onClose,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final barColour = t.shape == ThemeShape.sharp ? t.surface2 : t.surface0;

    final active = tabs.children[tabs.active];

    return Column(
      children: [
        Container(
          height: _headerStripHeight,
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
                          panel: tabs.children[i].panel,
                          title: tabs.children[i].panel.title,
                          active: i == tabs.active,
                          drag: drag,
                          onClose: onClose,
                          onPressed: () {
                            tabs.active = i;
                            onChanged();
                          },
                        ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
        // Two requirements meet here, each once lost to the other. Panel
        // state — scroll offsets, twirl-downs — survives a tab switch, so a
        // hidden tab's subtree stays MOUNTED (the TF round 5 fix, pinned by
        // dock_panel_state_test). And a hidden tab is never BUILT (Airyzz's
        // "dont build invisible panels", restored after K-182's merge
        // overwrote it): not at all before it is first shown, and not again
        // while hidden — _KeepAlivePane returns the same built instance, and
        // an identical child short-circuits Flutter's rebuild, so the dock
        // rebuilding sixty times a second never reaches a hidden panel.
        Expanded(
          child: Stack(
            children: [
              for (final tab in tabs.children)
                _paneBody(context, tab.panel, identical(tab, active)),
            ],
          ),
        ),
      ],
    );
  }

  /// One tab's body. Keyed per panel so reordering the tabs never
  /// cross-matches one panel's State onto another.
  Widget _paneBody(BuildContext context, Panel panel, bool visible) {
    return KeyedSubtree(
      key: ValueKey(panel),
      child: _KeepAlivePane(
        visible: visible,
        builder: (context) => _PaneChrome(
          panel: panel,
          activePanel: activePanel,
          drag: drag,
          child: buildPanel(context, panel),
        ),
      ),
    );
  }
}

/// A tab body that is mounted always, built only while visible.
///
/// While visible it rebuilds normally. While hidden it returns the widget
/// instance it last built — Flutter skips rebuilding an identical child, so
/// no build work cascades into hidden panels (their own listeners still fire;
/// that is each panel's business) — and a tab never yet shown builds nothing
/// at all. The subtree stays in the tree offstage with its ticker paused, so
/// State (scroll, twirls, search text) survives the flip either way.
class _KeepAlivePane extends StatefulWidget {
  final bool visible;
  final WidgetBuilder builder;
  const _KeepAlivePane({required this.visible, required this.builder});

  @override
  State<_KeepAlivePane> createState() => _KeepAlivePaneState();
}

class _KeepAlivePaneState extends State<_KeepAlivePane> {
  Widget? _built;

  @override
  Widget build(BuildContext context) {
    if (widget.visible) _built = widget.builder(context);
    final built = _built;
    if (built == null) return const SizedBox.shrink();
    return Offstage(
      offstage: !widget.visible,
      child: TickerMode(enabled: widget.visible, child: built),
    );
  }
}

class _TabPill extends StatefulWidget {
  final Panel panel;
  final String title;
  final bool active;
  final VoidCallback onPressed;
  final _DragController drag;
  final void Function(Panel) onClose;

  const _TabPill({
    required this.panel,
    required this.title,
    required this.active,
    required this.onPressed,
    required this.drag,
    required this.onClose,
  });

  @override
  State<_TabPill> createState() => _TabPillState();
}

class _TabPillState extends State<_TabPill> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final round = t.shape == ThemeShape.round;
    final Color fill;
    final Color textColour;
    if (widget.active) {
      // Round keeps its filled accent pill (K-394, §12.1). **Sharp draws no
      // box at all**: the mockups' `.kick.on` is bare text on the strip's own
      // grey — transparent fill, no border — and which tab is fronted reads
      // from the word brightening to `text_primary` alone. It had worn an
      // accent outline, which spends the accent on a resting state and makes
      // the strip's one lit tab look like a control to press.
      fill = round ? t.accent : const Color(0x00000000);
      textColour = round ? t.surface0 : t.textPrimary;
    } else if (_hover) {
      fill = t.surface3;
      textColour = t.textPrimary;
    } else {
      fill = const Color(0x00000000);
      textColour = t.textMuted;
    }
    // Always a border, transparent unless hover has something to show: a
    // border insets its child, so letting one appear would shrink the pill by
    // 2 px and shuffle every tab beside it as the pointer crossed the strip.
    final border = Border.all(
      color:
          _hover && !widget.active ? t.hairlineStrong : const Color(0x00000000),
      width: 1,
    );
    // A panel's name is a container label, so it is a kicker (§7.1, K-438):
    // one size, one weight, capitals applied here rather than in the arb file.
    // Which tab is fronted reads from the colour and the accent tick alone —
    // never from a bigger or heavier word, which would shuffle the strip every
    // time the front tab changed.
    final style =
        (widget.active ? t.kickerOn : t.kicker).copyWith(color: textColour);
    final label = Text(widget.title.toUpperCase(), style: style);
    final pill = Container(
      margin: const EdgeInsets.symmetric(horizontal: 2, vertical: 3),
      padding: const EdgeInsets.symmetric(horizontal: 8),
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: fill,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: border,
      ),
      child: round
          ? Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                // Accent, except on the pill that is itself filled with the
                // accent — the label flips there for the same reason, and an
                // accent dot on an accent field is not a dot. It is the same
                // mark either way; nothing about it reports state.
                _HeaderDot(
                    text: style, colour: widget.active ? textColour : t.accent),
                const SizedBox(width: 5),
                label,
              ],
            )
          : label,
    );
    return _DragSource(
      panel: widget.panel,
      drag: widget.drag,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          onTap: widget.onPressed,
          onSecondaryTapUp: (d) => _showTabMenu(context, d.globalPosition),
          // While this pill is the dragged one, it paints nothing but keeps
          // its footprint — egui leaves the gap while the ghost floats free.
          child: AnimatedBuilder(
            animation: widget.drag,
            builder: (context, child) => Opacity(
              opacity: widget.drag.dragged == widget.panel ? 0.0 : 1.0,
              child: child,
            ),
            child: pill,
          ),
        ),
      ),
    );
  }

  /// The tab's right-click menu (K-521): close the panel, and the pop-out that
  /// is not built yet, listed disabled rather than left off.
  ///
  /// **Pop out is greyed and says why.** Tearing a panel into its own window
  /// needs real operating-system windows, and Flutter has not shipped those on
  /// a stable release (K-449, `docs/impl/multi-window.md`) — so the row names
  /// the gate in its tooltip instead of quietly doing something else. It is
  /// deliberately *not* faked with a floating in-window panel: a panel that
  /// says it popped out and then cannot leave the app window is a worse answer
  /// than a disabled row.
  void _showTabMenu(BuildContext context, Offset position) {
    final t = ThemeScope.of(context).theme;
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              MenuRow(
                key: const ValueKey('tab-menu-close'),
                onPressed: () {
                  close(null);
                  widget.onClose(widget.panel);
                },
                child: Text(l10n.closePanel),
              ),
              LumitTooltip(
                message: l10n.popOutPanelBlocked,
                child: Padding(
                  key: const ValueKey('tab-menu-pop-out'),
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                  child: Text(
                    l10n.popOutPanel,
                    style: t.body.copyWith(color: t.textDisabled),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The pane body chrome: Sharp draws edge-to-edge on `surface1`; Round wraps
/// the content in a rounded, shadowed, padded card (dock.rs::pane_ui). Any
/// click inside makes this the active panel, which wears the accent boundary
/// (Shell::active_panel). A live re-dock drag paints the drop-zone preview
/// over the hovered pane.
class _PaneChrome extends StatelessWidget {
  final Panel panel;
  final ValueNotifier<Panel?> activePanel;
  final _DragController drag;
  final Widget child;

  const _PaneChrome({
    required this.panel,
    required this.activePanel,
    required this.drag,
    required this.child,
  });

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
          child: Container(
            key: drag.paneKeys[panel],
            decoration: BoxDecoration(
              color: t.surface1,
              borderRadius:
                  round ? BorderRadius.circular(t.tokens.cardRadius) : null,
              boxShadow: round ? t.tokens.cardShadow : null,
            ),
            // The accent boundary paints over the content's edge, like the
            // egui overlay stroke at Order::Middle. It is ALWAYS supplied — an
            // inactive pane wears a fully transparent border of the same width,
            // so the composed widget chain (the internal DecoratedBox) keeps a
            // constant shape across the active flip. A null-vs-non-null
            // foregroundDecoration would add or remove a layer, forcing Flutter
            // to discard the pane's Element subtree and rebuild it with fresh
            // State — losing scroll offsets and the gesture recogniser the
            // activating pointer-down had just armed (the first-click-drag bug).
            foregroundDecoration: BoxDecoration(
              border: Border.all(
                color:
                    active == panel ? t.accent : t.accent.withValues(alpha: 0),
                width: 1,
              ),
              borderRadius:
                  round ? BorderRadius.circular(t.tokens.cardRadius) : null,
            ),
            padding: round ? EdgeInsets.all(t.tokens.cardPadding) : null,
            clipBehavior: round ? Clip.antiAlias : Clip.none,
            child: Stack(
              children: [
                // **Nothing paints outside its pane** (K-451, step 5): below
                // the panel's declared minimum the content keeps that width
                // and slides. Wrapped here rather than in each panel so a
                // panel that has not thought about narrow widths still cannot
                // overflow — and so there is one place to read the rule.
                PanelFloor(minWidth: panelMinWidth(panel), child: child),
                Positioned.fill(child: _DropPreview(panel: panel, drag: drag)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The translucent accent region shown over the pane the pointer hovers while
/// a re-dock drag is live: the whole pane for a stack, the near half for an
/// edge split.
class _DropPreview extends StatelessWidget {
  final Panel panel;
  final _DragController drag;
  const _DropPreview({required this.panel, required this.drag});

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: drag,
        builder: (context, _) {
          if (drag.dragged == null ||
              drag.hoveredPanel != panel ||
              drag.dropPosition == null) {
            return const SizedBox.shrink();
          }
          final t = ThemeScope.of(context).theme;
          return IgnorePointer(
            child: CustomPaint(
              painter: _DropPainter(pos: drag.dropPosition!, accent: t.accent),
            ),
          );
        },
      );
}

class _DropPainter extends CustomPainter {
  final DropPosition pos;
  final Color accent;
  const _DropPainter({required this.pos, required this.accent});

  @override
  void paint(Canvas canvas, Size size) {
    final region = switch (pos) {
      DropPosition.stack => Offset.zero & size,
      DropPosition.left => Rect.fromLTWH(0, 0, size.width / 2, size.height),
      DropPosition.right =>
        Rect.fromLTWH(size.width / 2, 0, size.width / 2, size.height),
      DropPosition.above => Rect.fromLTWH(0, 0, size.width, size.height / 2),
      DropPosition.below =>
        Rect.fromLTWH(0, size.height / 2, size.width, size.height / 2),
    };
    canvas.drawRect(region, Paint()..color = accent.withValues(alpha: 0.35));
    canvas.drawRect(
      region,
      Paint()
        ..color = accent
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_DropPainter old) =>
      old.pos != pos || old.accent != accent;
}

/// **The bare-pane corner grip is gone** (owner review, 2026-08-24).
///
/// A solo pane used to carry a 16px dot-grid square in its top-right corner
/// (dock.rs::paint_bare_pane_grip), which dragged the pane's panel exactly like
/// a tab. It read as a control on every panel that had one, and on the Viewer it
/// sat over the right-hand end of that panel's own header strip — so the one
/// dock affordance drawn on the picture was also the one covering a picker.
///
/// What still carries re-docking: a **tab pill** drags its panel (`_TabPill`),
/// every pane is still a **drop target** (`_DropPreview`), and Window →
/// Workspace holds the presets, the reset and the per-panel toggles. A pane
/// that is alone in its slot can no longer be lifted; the natural home for that,
/// if it is wanted back, is the panel's own header strip rather than a mark
/// floating over its content.
