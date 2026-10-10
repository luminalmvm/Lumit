// The flowchart: a composition with the comps that place it on its left and
// the comps it places on its right. Tab opens it on the pointer, the way After
// Effects opens its mini-flowchart.
//
// One step each way. A neighbour whose nesting carries on wears a stub, and
// arrowing past the edge slides the chart along to it.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../src/rust/api/composition.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'fx_console_frb.dart' show lastKnownPointerPosition;

/// A comp's neighbours, or null when the comp has gone. The engine's answer
/// unless a test hands in its own.
typedef FlowchartRead = BridgeCompFlow? Function(CompositionReference comp);

BridgeCompFlow? _engineFlow(CompositionReference comp) {
  try {
    return comp.getFlow();
  } catch (_) {
    return null;
  }
}

/// Open the flowchart for the fronted comp, on the pointer. False when there
/// is no comp to draw.
bool openFlowchartFrb(BuildContext context, LumitUiState ui) {
  final comp = ui.selectedComp;
  if (comp == null) return false;
  return showFlowchartFrb(
    context: context,
    comp: comp,
    anchor: lastKnownPointerPosition,
    keyHint: ui.keymap.chordFor('comp.flowchart'),
    onOpen: ui.setSelectedComp,
  );
}

/// Show the chart with [comp] in the middle, its pill on [anchor] (window
/// coordinates, the middle of the window when null). [onOpen] fronts the comp
/// that was chosen.
bool showFlowchartFrb({
  required BuildContext context,
  required CompositionReference comp,
  required ValueChanged<CompositionReference> onOpen,
  Offset? anchor,
  String? keyHint,
  FlowchartRead read = _engineFlow,
}) {
  final flow = read(comp);
  if (flow == null) return false;
  final overlay = Overlay.of(context);
  late OverlayEntry entry;
  var open = true;
  void close() {
    if (!open) return;
    open = false;
    entry.remove();
  }

  final at = anchor == null ? null : overlayLocal(context, anchor);
  entry = OverlayEntry(
    // It fades up as a menu does, and takes the keyboard from its first
    // frame.
    builder: (context) => Entrance(
      spec: ThemeScope.of(context).motion.popup,
      leads: true,
      child: _Flowchart(
        comp: comp,
        flow: flow,
        read: read,
        anchor: at,
        keyHint: keyHint,
        onOpen: onOpen,
        onClose: close,
      ),
    ),
  );
  overlay.insert(entry);
  return true;
}

/// Which column the cursor is in.
enum _Side { usedBy, centre, uses }

const double _pillHeight = 24;
const double _pillGap = 6;
const double _pillPad = 10;
const double _pillMin = 72;
const double _pillMax = 200;
// The room between two columns, which the wires cross.
const double _wire = 44;
// The room outside a column for the stub that says the nesting carries on.
const double _stub = 16;
const double _pad = 12;
const double _edge = 1;
const double _kickerHeight = 20;
const double _footHeight = 20;
const double _margin = 8;

class _Flowchart extends StatefulWidget {
  final CompositionReference comp;
  final BridgeCompFlow flow;
  final FlowchartRead read;
  final Offset? anchor;
  final String? keyHint;
  final ValueChanged<CompositionReference> onOpen;
  final VoidCallback onClose;

  const _Flowchart({
    required this.comp,
    required this.flow,
    required this.read,
    required this.anchor,
    required this.keyHint,
    required this.onOpen,
    required this.onClose,
  });

  @override
  State<_Flowchart> createState() => _FlowchartState();
}

class _FlowchartState extends State<_Flowchart> {
  late CompositionReference _comp = widget.comp;
  late BridgeCompFlow _flow = widget.flow;
  _Side _side = _Side.centre;
  int _row = 0;

  final FocusNode _focus = FocusNode(debugLabel: 'flowchart');
  final ScrollController _scroll = ScrollController();
  VoidCallback? _escapeRelease;

  @override
  void initState() {
    super.initState();
    // The keyboard is the chart's while it is up, as it is the console's.
    _escapeRelease = EscapeLadder.register(EscapeRung.dialog, () {
      widget.onClose();
      return true;
    });
    markModalMounted();
    _focus.addListener(_keepFocus);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _escapeRelease?.call();
    markModalUnmounted();
    _focus
      ..removeListener(_keepFocus)
      ..dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _keepFocus() {
    if (mounted && !_focus.hasFocus) _focus.requestFocus();
  }

  List<BridgeCompFlowLink> _links(_Side side) => switch (side) {
        _Side.usedBy => _flow.usedBy,
        _Side.uses => _flow.uses,
        _Side.centre => const [],
      };

  /// The neighbour under the cursor, or null when the cursor is on the middle.
  BridgeCompFlowLink? get _cursor {
    final links = _links(_side);
    return links.isEmpty ? null : links[_row.clamp(0, links.length - 1)];
  }

  void _moveTo(_Side side, int row) {
    if (side != _Side.centre && _links(side).isEmpty) return;
    if (side == _side && row == _row) return;
    setState(() {
      _side = side;
      _row = row;
    });
    WidgetsBinding.instance.addPostFrameCallback((_) => _reveal());
  }

  /// Slide the chart so [link] is the middle, keeping the cursor on it.
  void _slideTo(BridgeCompFlowLink link) {
    final flow = widget.read(link.comp);
    if (flow == null) return;
    setState(() {
      _comp = link.comp;
      _flow = flow;
      _side = _Side.centre;
      _row = 0;
    });
  }

  /// One step along the nesting: towards the comps that place this one, or
  /// towards the ones it places.
  void _step(_Side towards) {
    if (_side == _Side.centre) {
      _moveTo(towards, 0);
    } else if (_side != towards) {
      _moveTo(_Side.centre, 0);
    } else if (_cursor case final link? when link.more) {
      _slideTo(link);
    }
  }

  void _open(CompositionReference comp) {
    widget.onClose();
    if (comp != widget.comp) widget.onOpen(comp);
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    final first = event is KeyDownEvent;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowLeft:
        _step(_Side.usedBy);
      case LogicalKeyboardKey.arrowRight:
        _step(_Side.uses);
      case LogicalKeyboardKey.arrowUp:
        if (_row > 0) _moveTo(_side, _row - 1);
      case LogicalKeyboardKey.arrowDown:
        if (_row < _links(_side).length - 1) _moveTo(_side, _row + 1);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (first) _open(_cursor?.comp ?? _comp);
      // The key that opened it shuts it. Not on a held key, which would
      // blink it.
      case LogicalKeyboardKey.tab:
        if (first) widget.onClose();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  double _columnWidth(LumitTheme t, Iterable<String> names) {
    var widest = 0.0;
    for (final name in names) {
      final painter = TextPainter(
        text: TextSpan(text: name, style: t.body),
        textDirection: TextDirection.ltr,
        maxLines: 1,
      )..layout();
      if (painter.width > widest) widest = painter.width;
      painter.dispose();
    }
    return (widest + (_pillPad + _edge) * 2)
        .ceilToDouble()
        .clamp(_pillMin, _pillMax);
  }

  _Geometry _geometry(LumitTheme t) {
    final left = _flow.usedBy, right = _flow.uses;
    final leftWidth =
        left.isEmpty ? 0.0 : _columnWidth(t, left.map((l) => l.name));
    final rightWidth =
        right.isEmpty ? 0.0 : _columnWidth(t, right.map((l) => l.name));
    final centreWidth = _columnWidth(t, [_flow.name]);
    final centreX = left.isEmpty ? 0.0 : _stub + leftWidth + _wire;
    final rightX = centreX + centreWidth + _wire;
    final rows = left.length > right.length ? left.length : right.length;
    return _Geometry(
      leftX: _stub,
      leftWidth: leftWidth,
      centreX: centreX,
      centreWidth: centreWidth,
      rightX: rightX,
      rightWidth: rightWidth,
      width:
          right.isEmpty ? centreX + centreWidth : rightX + rightWidth + _stub,
      height: (rows < 1 ? 1 : rows) * (_pillHeight + _pillGap) - _pillGap,
      leftRows: left.length,
      rightRows: right.length,
    );
  }

  /// Scroll a tall chart so the cursor's pill is on screen.
  void _reveal() {
    if (!mounted || !_scroll.hasClients || _side == _Side.centre) return;
    final g = _geometry(ThemeScope.of(context).theme);
    final top = g.rowTop(_links(_side).length, _row);
    final view = _scroll.position;
    if (top < view.pixels) {
      _scroll.jumpTo(top);
    } else if (top + _pillHeight > view.pixels + view.viewportDimension) {
      _scroll.jumpTo(top + _pillHeight - view.viewportDimension);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final g = _geometry(t);
    final alone = _flow.usedBy.isEmpty && _flow.uses.isEmpty;
    final head = alone ? 0.0 : _kickerHeight;
    return Focus(
      focusNode: _focus,
      onKeyEvent: _onKey,
      child: LayoutBuilder(builder: (context, box) {
        final width = g.width + (_pad + _edge) * 2;
        // Everything in the popover but the chart: its edges and padding,
        // the kickers, the hairline and the foot.
        final fixed = (_pad + _edge) * 2 + head + _edge + _footHeight;
        final room = box.maxHeight - _margin * 2 - fixed;
        final chart = g.height < room ? g.height : (room < 0 ? 0.0 : room);
        final anchor =
            widget.anchor ?? Offset(box.maxWidth / 2, box.maxHeight / 2);
        // The middle pill lands on the pointer, pulled in to stay on screen.
        final left = _fit(
            anchor.dx - _edge - _pad - g.centreX - g.centreWidth / 2,
            _margin,
            box.maxWidth - width - _margin);
        final top = _fit(anchor.dy - _edge - _pad - head - chart / 2, _margin,
            box.maxHeight - _margin - fixed - chart);
        return Stack(children: [
          Positioned.fill(
            key: const ValueKey('flowchart-away'),
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: widget.onClose,
              onSecondaryTap: widget.onClose,
            ),
          ),
          Positioned(
            key: const ValueKey('flowchart'),
            left: left,
            top: top,
            width: width,
            child: Container(
              decoration: BoxDecoration(
                color: t.surface1,
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                border: Border.all(color: t.hairline, width: _edge),
                boxShadow: t.floatShadow,
              ),
              child: Entrance.content(
                spec: ThemeScope.of(context).motion.content,
                rise: ThemeScope.of(context).motion.contentRise,
                blur: ThemeScope.of(context).motion.contentBlur,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Padding(
                      padding: const EdgeInsets.fromLTRB(_pad, _pad, _pad, 0),
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          if (!alone) _kickers(t, g),
                          SizedBox(
                            height: chart,
                            child: SingleChildScrollView(
                              controller: _scroll,
                              child: _chart(t, g),
                            ),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(height: _pad),
                    Container(height: _edge, color: t.hairline),
                    _foot(t, alone),
                  ],
                ),
              ),
            ),
          ),
        ]);
      }),
    );
  }

  static double _fit(double v, double lo, double hi) =>
      hi < lo ? lo : (v < lo ? lo : (v > hi ? hi : v));

  /// The words over the two outer columns.
  Widget _kickers(LumitTheme t, _Geometry g) => SizedBox(
        width: g.width,
        height: _kickerHeight,
        child: Stack(children: [
          if (g.leftRows > 0)
            Positioned(
              left: g.leftX,
              width: g.leftWidth,
              child: Text(t.kickerCase(l10n.flowchartUsedIn),
                  overflow: TextOverflow.ellipsis, style: t.kicker),
            ),
          if (g.rightRows > 0)
            Positioned(
              left: g.rightX,
              width: g.rightWidth,
              child: Text(t.kickerCase(l10n.flowchartContains),
                  overflow: TextOverflow.ellipsis, style: t.kicker),
            ),
        ]),
      );

  Widget _chart(LumitTheme t, _Geometry g) => SizedBox(
        width: g.width,
        height: g.height,
        child: Stack(children: [
          Positioned.fill(
            child: CustomPaint(
              painter: _WirePainter(
                geometry: g,
                leftMore: [for (final l in _flow.usedBy) l.more],
                rightMore: [for (final l in _flow.uses) l.more],
                side: _side,
                row: _row,
                rest: t.hairlineStrong,
                hot: t.accent,
              ),
            ),
          ),
          _pill(t,
              key: const ValueKey('flowchart-centre'),
              left: g.centreX,
              top: g.rowTop(1, 0),
              width: g.centreWidth,
              name: _flow.name,
              centre: true,
              hot: _side == _Side.centre,
              onHover: () => _moveTo(_Side.centre, 0),
              onTap: () => _open(_comp)),
          for (final side in const [_Side.usedBy, _Side.uses])
            for (final (i, link) in _links(side).indexed) ...[
              _pill(t,
                  key: ValueKey('flowchart-${side.name}-$i'),
                  left: side == _Side.usedBy ? g.leftX : g.rightX,
                  top: g.rowTop(_links(side).length, i),
                  width: side == _Side.usedBy ? g.leftWidth : g.rightWidth,
                  name: link.name,
                  hot: _side == side && _row == i,
                  onHover: () => _moveTo(side, i),
                  onTap: () => _open(link.comp)),
              // The stub slides the chart along without opening anything.
              if (link.more)
                Positioned(
                  key: ValueKey('flowchart-more-${side.name}-$i'),
                  left: side == _Side.usedBy ? 0 : g.rightX + g.rightWidth,
                  top: g.rowTop(_links(side).length, i),
                  width: _stub,
                  height: _pillHeight,
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: () => _slideTo(link),
                  ),
                ),
            ],
        ]),
      );

  Widget _pill(
    LumitTheme t, {
    required Key key,
    required double left,
    required double top,
    required double width,
    required String name,
    required bool hot,
    required VoidCallback onHover,
    required VoidCallback onTap,
    bool centre = false,
  }) {
    // The middle is the accent's: full while the cursor is on it, a tint once
    // the cursor has moved off. A neighbour lifts under the cursor.
    final fill = centre
        ? (hot ? t.accent : t.accent.withValues(alpha: 0.16))
        : (hot ? t.surface3 : null);
    final edge = centre ? t.accent : (hot ? t.hairlineStrong : t.hairline);
    final style = centre && hot
        ? t.body.copyWith(color: accentInk(t))
        : (hot || centre ? t.bodyPrimary : t.body);
    return Positioned(
      key: key,
      left: left,
      top: top,
      width: width,
      height: _pillHeight,
      child: MouseRegion(
        // A move, not an enter, so a chart sliding under a still pointer
        // leaves the cursor where the keys put it.
        onHover: (_) => onHover(),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: Container(
            alignment: Alignment.center,
            padding: const EdgeInsets.symmetric(horizontal: _pillPad),
            decoration: BoxDecoration(
              color: fill,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              border: Border.all(color: edge, width: 1),
            ),
            child: Text(name,
                maxLines: 1, overflow: TextOverflow.ellipsis, style: style),
          ),
        ),
      ),
    );
  }

  Widget _foot(LumitTheme t, bool alone) => Container(
        height: _footHeight,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Row(children: [
          Expanded(
            child: Text(
              alone ? l10n.flowchartAlone : l10n.flowchartOpens,
              key: const ValueKey('flowchart-foot'),
              overflow: TextOverflow.ellipsis,
              style: t.kicker,
            ),
          ),
          if (widget.keyHint case final hint?) ...[
            const SizedBox(width: 8),
            Text(hint, style: t.kicker),
          ],
        ]),
      );
}

/// Where everything in the chart sits, worked out before layout so the wires
/// and the pills agree.
class _Geometry {
  final double leftX, leftWidth, centreX, centreWidth, rightX, rightWidth;
  final double width, height;
  final int leftRows, rightRows;

  const _Geometry({
    required this.leftX,
    required this.leftWidth,
    required this.centreX,
    required this.centreWidth,
    required this.rightX,
    required this.rightWidth,
    required this.width,
    required this.height,
    required this.leftRows,
    required this.rightRows,
  });

  /// The top of row [row] in a column of [rows], each column centred on the
  /// chart's height.
  double rowTop(int rows, int row) {
    final column = rows * (_pillHeight + _pillGap) - _pillGap;
    return (height - column) / 2 + row * (_pillHeight + _pillGap);
  }

  @override
  bool operator ==(Object other) =>
      other is _Geometry &&
      other.leftWidth == leftWidth &&
      other.centreWidth == centreWidth &&
      other.rightWidth == rightWidth &&
      other.leftRows == leftRows &&
      other.rightRows == rightRows;

  @override
  int get hashCode =>
      Object.hash(leftWidth, centreWidth, rightWidth, leftRows, rightRows);
}

/// The wires from the middle pill to each neighbour, and the stubs past the
/// ones whose nesting carries on.
class _WirePainter extends CustomPainter {
  final _Geometry geometry;
  final List<bool> leftMore, rightMore;
  final _Side side;
  final int row;
  final Color rest, hot;

  const _WirePainter({
    required this.geometry,
    required this.leftMore,
    required this.rightMore,
    required this.side,
    required this.row,
    required this.rest,
    required this.hot,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final g = geometry;
    final middle = g.rowTop(1, 0) + _pillHeight / 2;
    void column(_Side which, List<bool> more) {
      final left = which == _Side.usedBy;
      // The pill's edge facing the middle, the middle's edge facing it, and
      // which way is outwards.
      final near = left ? g.leftX + g.leftWidth : g.rightX;
      final centre = left ? g.centreX : g.centreX + g.centreWidth;
      final outer = left ? g.leftX : g.rightX + g.rightWidth;
      final out = left ? -1.0 : 1.0;
      // The cursor's wire last, so it lies over the others.
      final order = [
        for (var i = 0; i < more.length; i++)
          if (!(side == which && row == i)) i,
        if (side == which && row < more.length) row,
      ];
      for (final i in order) {
        final paint = Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1
          ..color = side == which && row == i ? hot : rest;
        final y = g.rowTop(more.length, i) + _pillHeight / 2;
        final bend = (near + centre) / 2;
        canvas.drawPath(
          Path()
            ..moveTo(centre, middle)
            ..cubicTo(bend, middle, bend, y, near, y),
          paint,
        );
        if (!more[i]) continue;
        final tip = outer + out * (_stub - 4);
        canvas.drawPath(
          Path()
            ..moveTo(outer, y)
            ..lineTo(tip, y)
            ..moveTo(tip - out * 4, y - 4)
            ..lineTo(tip, y)
            ..lineTo(tip - out * 4, y + 4),
          paint,
        );
      }
    }

    column(_Side.usedBy, leftMore);
    column(_Side.uses, rightMore);
  }

  @override
  bool shouldRepaint(_WirePainter old) =>
      old.geometry != geometry ||
      old.side != side ||
      old.row != row ||
      old.rest != rest ||
      old.hot != hot ||
      !_same(old.leftMore, leftMore) ||
      !_same(old.rightMore, rightMore);

  static bool _same(List<bool> a, List<bool> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}
