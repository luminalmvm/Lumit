// One box on a node canvas: what it shows, where its rows and sockets sit,
// and how it is drawn. The layer graph, a node graph composition and the
// Node panel all lay out and draw their boxes with these.

import 'dart:math' as math;

import 'package:flutter/gestures.dart' show kSecondaryMouseButton;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';

import '../icons/icons.dart' show LumitIcon, lumitIcon;
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'fx_section.dart' show fxEnableMark;
import 'graph_panel.dart';
import 'node_menus.dart' show graphPortWord;

/// The picture of its inner graph a Custom shader box carries under its rows:
/// how tall it is, the air round it, and the room the two take on the box.
const double graphShaderThumbHeight = 56;
const double graphFootPad = 6;
const double graphShaderFoot = graphShaderThumbHeight + graphFootPad * 2;

/// What one box shows, whichever graph it came out of.
///
/// The canvas draws three subjects and each has a model of its own, so a card
/// is drawn from this record rather than from any one of them: the key is what
/// the panel means by identity everywhere else, and the three marks say which
/// of the header's controls this box has.
typedef GraphCard = ({
  String key,
  String title,
  String? customName,

  /// A word after the names saying what the box is: a Read box's item kind, an
  /// Input's kind. Null where the title says it all.
  String? kicker,
  bool enabled,

  /// The bypass tick, the twirl, and whether a double-click renames.
  bool tick,
  bool twirl,
  bool rename,

  /// A box with an inside wears the viz tint on its header.
  bool tinted,
  List<BridgePort> inputs,
  List<BridgePort> outputs,

  /// A box that reads a points stream with none wired in wears a warning.
  bool noStream,
});

/// One row of a box: the input socket on its left edge, the output socket on
/// its right, and on an open node graph box the parameter whose control fills
/// it. [top] is measured from under the header.
typedef GraphRow = ({
  BridgePort? input,
  BridgePort? output,
  String? param,

  /// A group's heading, on a row of its own above the group's first row.
  String? heading,
  double top,
  double height,
});

/// One box, laid out: where it sits and which sockets it shows.
class GraphBox {
  final GraphCard card;
  final Rect rect;
  final List<BridgePort> inputs;
  final List<BridgePort> outputs;
  final List<GraphRow> rows;

  /// The label chip its header is tinted with, 0 for none.
  final int colour;

  /// Folded down to its header: its wires meet at the header's two ends.
  final bool collapsed;
  const GraphBox(this.card, this.rect, this.inputs, this.outputs, this.rows,
      {this.colour = 0, this.collapsed = false});

  String get key => card.key;

  /// Whether the box draws a control on any row, which an open box does.
  bool get hasControls => rows.any((r) => r.param != null);

  Offset? socket(String portId, bool isInput) {
    if (collapsed) {
      final ports = isInput ? inputs : outputs;
      if (!ports.any((p) => p.id == portId)) return null;
      return Offset(isInput ? rect.left : rect.right,
          rect.top + 1 + graphNodeHeaderHeight / 2);
    }
    for (final row in rows) {
      if ((isInput ? row.input : row.output)?.id != portId) continue;
      return Offset(
        isInput ? rect.left : rect.right,
        rect.top + 1 + graphNodeHeaderHeight + row.top + row.height / 2,
      );
    }
    return null;
  }
}

/// One box's rectangle and the sockets it draws, from its card and the spot it
/// sits on.
///
/// A shut box draws the picture's own path and whatever is already wired; open,
/// it draws every socket it has (§1.4). Exposure grows the box; it is not a
/// second kind of wiring. [params] are the parameters the box gives a row
/// each, in the order the panel lists them, and [kickers] the group heading
/// drawn before a row, by the row's id. [foot] is room left under the rows,
/// which a Custom shader box fills with its inner graph.
GraphBox graphLayoutBox(
  GraphCard card,
  Offset at, {
  required bool open,
  double width = graphNodeWidth,
  List<String> params = const [],
  Map<String, String> kickers = const {},
  double foot = 0,
  int colour = 0,
  bool collapsed = false,
}) {
  // Folded, a box is its header, and the sockets a shut box draws all meet
  // at the header's two ends.
  if (collapsed) {
    bool kept(BridgePort p) => _alwaysDrawn(p.portType) || p.wired;
    return GraphBox(
      card,
      Rect.fromLTWH(at.dx, at.dy, width + 2, 2 + graphNodeHeaderHeight),
      [for (final p in card.inputs) if (kept(p)) p],
      [for (final p in card.outputs) if (kept(p)) p],
      const [],
      colour: colour,
      collapsed: true,
    );
  }
  // Open, a parameter socket draws with its row, so a group the schema
  // folds shut stays folded on the box too. A box with no rows at all, a
  // Read or the Output, draws every socket it has.
  final wanted = params.toSet();
  List<BridgePort> shown(List<BridgePort> ports, {bool inputs = false}) => [
        for (final p in ports)
          if (_alwaysDrawn(p.portType) ||
              p.wired ||
              (open && (!inputs || wanted.isEmpty || wanted.contains(p.id))))
            p,
      ];
  final inputs = shown(card.inputs, inputs: true);
  final outputs = shown(card.outputs);
  final rows = graphRows(inputs, outputs, params, kickers);
  final height = rows.isEmpty ? 0.0 : rows.last.top + rows.last.height;
  return GraphBox(
    card,
    Rect.fromLTWH(
      at.dx,
      at.dy,
      width + 2,
      2 + graphNodeHeaderHeight + height + foot,
    ),
    inputs,
    outputs,
    rows,
    colour: colour,
  );
}

/// The rows a box draws, top to bottom.
///
/// The picture's sockets come first in the order the engine lists them, then
/// one control row per parameter where the first parameter socket stood, then
/// whatever sockets follow. An output shares a socket's row where there is one
/// to share, else a control's, and only a box whose every row already carries
/// an output grows another.
List<GraphRow> graphRows(
  List<BridgePort> inputs,
  List<BridgePort> outputs,
  List<String> params, [
  Map<String, String> kickers = const {},
]) {
  final byId = {for (final p in inputs) p.id: p};
  final wanted = params.toSet();
  final first = inputs.indexWhere((p) => wanted.contains(p.id));
  final rows = <({
    BridgePort? input,
    BridgePort? output,
    String? param,
    String? heading,
  })>[];
  // A group's heading takes a row of its own, above the group's first.
  void controls() => rows.addAll([
        for (final id in params) ...[
          if (kickers[id] case final heading?)
            (input: null, output: null, param: null, heading: heading),
          (input: byId[id], output: null, param: id, heading: null),
        ],
      ]);
  for (var i = 0; i < inputs.length; i++) {
    if (i == first) controls();
    if (!wanted.contains(inputs[i].id)) {
      rows.add((input: inputs[i], output: null, param: null, heading: null));
    }
  }
  if (first < 0) controls();
  var at = 0;
  for (final out in outputs) {
    while (at < rows.length &&
        (rows[at].param != null || rows[at].heading != null)) {
      at++;
    }
    if (at < rows.length) {
      rows[at] =
          (input: rows[at].input, output: out, param: null, heading: null);
      at++;
      continue;
    }
    // No socket row left to share with. A box of controls alone, which is
    // every driver, puts its output on the first row with none yet rather than
    // on a blank row under everything it draws.
    final spare =
        rows.indexWhere((r) => r.output == null && r.heading == null);
    if (spare < 0) {
      rows.add((input: null, output: out, param: null, heading: null));
      at = rows.length;
    } else {
      rows[spare] = (
        input: rows[spare].input,
        output: out,
        param: rows[spare].param,
        heading: null,
      );
    }
  }
  final laid = <GraphRow>[];
  var top = 0.0;
  for (final row in rows) {
    const height = graphPortRowHeight;
    laid.add((
      input: row.input,
      output: row.output,
      param: row.param,
      heading: row.heading,
      top: top,
      height: height,
    ));
    top += height;
  }
  return laid;
}

/// Sockets that are always drawn, wired or not: the picture's own path, and
/// every **wire-only** port beside it.
///
/// A wire-only port has no stored value and therefore no panel row anywhere
/// (points-stream.md §2.2) — a matte's, an audio stream's, a points stream's.
/// The socket is the only way to reach it, so hiding it behind the twirl
/// would hide the whole port. A *parameter* socket is different: the row is
/// still there in Effect controls, so the canvas may keep it folded away.
bool _alwaysDrawn(BridgePortType type) =>
    type == BridgePortType.image ||
    type == BridgePortType.matte ||
    type == BridgePortType.audio ||
    type == BridgePortType.points;

/// One box on the canvas: a header strip carrying its enable tick, its twirl
/// and its name, then the port rows with their sockets sitting on the border.
///
/// Public because all three of the panel's subjects draw the same card: what a
/// box shows is [GraphCard], and which model it came out of is the caller's
/// business alone.
class GraphNodeCard extends StatelessWidget {
  final GraphBox box;
  final bool selected;
  final bool exposed;
  final VoidCallback onExpose;
  final VoidCallback onBypass;

  /// A control on this card has taken the press, so the canvas behind it
  /// leaves the pick alone.
  final VoidCallback onOwnPress;

  /// The name is an inline editor rather than a label, its commit —
  /// empty clears back to the box's own label — and the Escape that throws the
  /// edit away instead. The same contract the Effect controls heading
  /// has, because it is the same rename.
  final bool renaming;
  final VoidCallback onStartRename;
  final ValueChanged<String> onRenamed;
  final VoidCallback onRenameCancelled;

  /// The control a parameter row draws, by parameter id: the Node panel's
  /// own row, on the box. Null on a canvas whose rows are sockets alone.
  final Widget Function(String param)? paramRow;

  /// The value a wired parameter row reads, drawn dimmed at the row's right
  /// beside the port name. Null where the row shows no value.
  final Widget? Function(String param)? drivenValue;

  /// What the box draws under its rows, in the room its layout left: a Custom
  /// shader's picture of its inner graph. A press on it is its own.
  final Widget? foot;

  /// The socket on this card a wire in hand has snapped to, drawn lit.
  final GraphSocket? snap;

  const GraphNodeCard({
    super.key,
    required this.box,
    required this.selected,
    required this.exposed,
    required this.onExpose,
    required this.onBypass,
    required this.onOwnPress,
    required this.renaming,
    required this.onStartRename,
    required this.onRenamed,
    required this.onRenameCancelled,
    this.paramRow,
    this.drivenValue,
    this.foot,
    this.snap,
  });

  GraphCard get _card => box.card;
  String get _key => box.key;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Sized by the canvas's Positioned, and hit-tested past its own edge so a
    // socket on the border takes a hover for its tooltip.
    return GraphBoxStack(
      key: ValueKey<String>('graph-node-$_key'),
      children: [
        Positioned.fill(
          child: GraphNodeFrame(
            // A bypassed box draws its border dashed, both drawings; the
            // selected one draws it in `animated`.
            colour: selected ? t.animated : t.hairline,
            dashed: !_card.enabled,
            fill: t.surface1,
            radius: t.tokens.controlRadius,
          ),
        ),
        Positioned(
          left: 1,
          top: 1,
          width: box.rect.width - 2,
          child: _header(t),
        ),
        // Folded, one socket at each end of the header stands for the
        // wires that meet there.
        if (box.collapsed) ...[
          if (box.inputs.isNotEmpty)
            Positioned(
              left: -graphSocketSize / 2,
              top: 1 + (graphNodeHeaderHeight - graphSocketSize) / 2,
              child: _socket(t, _lead(box.inputs), isInput: true),
            ),
          if (box.outputs.isNotEmpty)
            Positioned(
              right: -graphSocketSize / 2,
              top: 1 + (graphNodeHeaderHeight - graphSocketSize) / 2,
              child: _socket(t, _lead(box.outputs), isInput: false),
            ),
        ],
        for (final row in box.rows)
          Positioned(
            left: 1,
            top: 1 + graphNodeHeaderHeight + row.top,
            width: box.rect.width - 2,
            height: row.height,
            child: _row(t, row),
          ),
        if (foot case final foot?)
          Positioned(
            left: 1 + graphFootPad,
            right: 1 + graphFootPad,
            bottom: 1 + graphFootPad,
            child: _claim(foot),
          ),
      ],
    );
  }

  /// The port a folded box's end socket draws as: a wired one if there is
  /// one, so the socket fills when anything meets there.
  BridgePort _lead(List<BridgePort> ports) =>
      ports.where((p) => p.wired).firstOrNull ?? ports.first;

  Widget _header(LumitTheme t) {
    // A box with an inside (a Custom shader, a nested node graph) wears the
    // viz tint so it reads at a glance; a box given a colour wears that.
    final ground = _card.tinted ? graphShaderHeader(t) : t.surface2;
    return Container(
        key: ValueKey<String>('graph-node-header-$_key'),
        height: graphNodeHeaderHeight,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(
          color: box.colour == 0
              ? ground
              : Color.alphaBlend(
                  t.labelColour(box.colour).withValues(alpha: 0.3), ground),
          // Folded, the header is the whole card and takes its corners.
          borderRadius: box.collapsed
              ? BorderRadius.circular(math.max(0.0, t.tokens.controlRadius - 1))
              : null,
          border: box.collapsed
              ? null
              : Border(bottom: BorderSide(color: t.hairline)),
        ),
        child: Row(
          children: [
            // **Enable tick, twirl, name** — the order an Effect controls
            // heading reads in, because on a node card it is the same
            // grammar and, for the tick, the same control. What the box *is
            // doing* comes before what the header does to what is under it.
            if (_card.tick) ...[
              _enable(t),
              const SizedBox(width: 2),
            ],
            // A **driver** draws every socket it has whatever its exposure
            // says, so a twirl on one would be a control that answers nothing
            // and it is left off.
            // Folded, there are no sockets for the twirl to open.
            if (_card.twirl && !box.collapsed) ...[
              _twirl(t),
              const SizedBox(width: 2),
            ],
            // **The name takes every pixel the marks leave** (owner, desk
            // test). It used to be a `Flexible` name beside a `Spacer`, and a
            // `Spacer` is an `Expanded` of flex 1: the two shared the header's
            // free space half each, so a name ellipsised with half the strip
            // standing empty beside it. One `Expanded` holding the names puts
            // the whole remainder at their disposal — the controls are
            // fixed-width — so a name cuts only when it is genuinely out of
            // room.
            Expanded(child: renaming ? _editor(t) : _names(t)),
            // **Nothing wired in**. A driver that reads a stream and
            // has none answers its documented no-op — a distance so large it
            // pins whatever it drives at the far end of the range — and the
            // box is the one place that can say so before the wire is drawn.
            if (_card.noStream) ...[
              const SizedBox(width: 4),
              LumitTooltip(
                message: l10n.graphNoStream,
                child: Container(
                  key: ValueKey<String>('graph-no-stream-$_key'),
                  width: graphBadgeSize,
                  height: graphBadgeSize,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                    border: Border.all(color: t.warning),
                  ),
                  child: Text('!',
                      style: t.mono.copyWith(fontSize: 9, color: t.warning)),
                ),
              ),
            ],
          ],
        ),
      );
  }

  /// The box's type name and, where the user has given it one, their own name
  /// beside it. **Double-click either to rename the box** (owner, desk test):
  /// the derived boxes are left out, since the Source shows the layer's name
  /// and the Out is the layer's own end — neither is a thing with a name of
  /// its own to give.
  Widget _names(LumitTheme t) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onDoubleTap: _card.rename ? onStartRename : null,
        child: Row(
          children: [
            Flexible(
              child: Text(
                _card.title,
                key: ValueKey<String>('graph-node-name-$_key'),
                style: _card.enabled ? t.kickerOn : t.kicker,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            // A box the user has named of their own says so beside its type
            // name, quieter and less tracked — the same reading the drawing
            // gives the Audio level box called "Music".
            if (_card.customName case final own?) ...[
              const SizedBox(width: 6),
              Flexible(
                child: Text(own,
                    style: t.kicker.copyWith(letterSpacing: 0.54),
                    overflow: TextOverflow.ellipsis),
              ),
            ],
            // And what kind of box it is, where the title does not say: the
            // item's kind under a Read, the value's kind under an Input.
            if (_card.kicker case final word?) ...[
              const SizedBox(width: 6),
              Flexible(
                child: Text(word,
                    key: ValueKey<String>('graph-node-kicker-$_key'),
                    style: t.kicker,
                    overflow: TextOverflow.ellipsis),
              ),
            ],
          ],
        ),
      );

  /// The inline rename, opened with the current name selected and committing
  /// on Enter or on clicking away — the contract every inline rename in the
  /// application has, and the one the Effect controls heading's own
  /// editor keeps.
  Widget _editor(LumitTheme t) => GraphNameField(
        key: ValueKey<String>('graph-node-rename-$_key'),
        initial: _card.customName ?? '',
        onDone: onRenamed,
        onCancel: onRenameCancelled,
      );

  /// **Pressing a control on the header is not picking the box** (the
  /// Timeline's rule for its switch cells). The canvas reads its
  /// pointers through one `Listener` above this card, and a child listener is
  /// dispatched first — so the flag is set before the canvas decides what the
  /// press meant. Without it, switching one of four picked boxes off collapsed
  /// the pick to that box and then bypassed only it.
  /// A right press is not the control's: it opens the box's menu.
  Widget _claim(Widget child) => Listener(
      onPointerDown: (e) {
        if (e.buttons != kSecondaryMouseButton) onOwnPress();
      },
      child: child);

  /// **The enable tick**, left of the name — the Effect controls heading's own
  /// switch face ([fxEnableMark]), because a box's enable and its effect
  /// card's switch are one control drawn on two surfaces. Off, the box's
  /// border goes dashed and its name goes quiet (15-DESIGN §12A.7): a bypassed
  /// box is not a fault, it is the one box deliberately doing nothing.
  Widget _enable(LumitTheme t) => _claim(SizedBox(
        width: graphEnableSize,
        height: graphEnableSize,
        child: Center(
          child: fxEnableMark(
            key: ValueKey<String>('graph-enable-$_key'),
            on: _card.enabled,
            onChanged: (_) => onBypass(),
          ),
        ),
      ));

  /// **The twirl**, which opens the box up. Shut, an effect draws the
  /// picture's own sockets and whatever is already wired; open, it draws a
  /// socket for every parameter it has (`LayerGraph::exposed`). The same mark
  /// and the same reading as the twirl on an Effect controls heading — what is
  /// under the header, shown or folded away.
  Widget _twirl(LumitTheme t) => _claim(HoverMark(
        builder: (context, lit) => GestureDetector(
          key: ValueKey<String>('graph-twirl-$_key'),
          behavior: HitTestBehavior.opaque,
          onTap: onExpose,
          child: SizedBox(
            width: graphBadgeSize,
            height: graphBadgeSize,
            child: Center(
              child: TwirlTurn(
                open: exposed,
                child: lumitIcon(
                  exposed ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                  size: graphTwirlSize,
                  color: exposed
                      ? t.textPrimary
                      : Color.lerp(t.textMuted, t.textPrimary, lit)!,
                ),
              ),
            ),
          ),
        ),
      ));

  /// One row: its sockets, and either the control that sets the parameter or
  /// the port's name. **A wired row draws the name alone**, because the wire
  /// is the value and a control under it would be a lie about what is in
  /// charge, the reading the Node panel's driven rows take.
  Widget _row(LumitTheme t, GraphRow row) {
    final input = row.input;
    final output = row.output;
    if (row.heading case final heading?) {
      return Padding(
        padding: const EdgeInsets.only(left: graphRowInset),
        child: Align(
          alignment: Alignment.centerLeft,
          child: Text(t.kickerCase(heading),
              style: t.kicker, overflow: TextOverflow.ellipsis),
        ),
      );
    }
    final control = row.param != null && !(input?.wired ?? false)
        ? paramRow?.call(row.param!)
        : null;
    final driven = row.param != null && (input?.wired ?? false)
        ? drivenValue?.call(row.param!)
        : null;
    // Hit-tested past its edge, as the card is: the socket sits half outside.
    return GraphBoxStack(
      children: [
        // A readout, not a control: a press on it is a press on the box.
        if (control != null)
          Positioned.fill(child: control)
        else if (input != null)
          Positioned.fill(
            child: Padding(
              padding: const EdgeInsets.only(left: graphRowInset),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(engineLabel(input.label),
                    key: ValueKey<String>(
                        'graph-port-$_key-in-${input.id}'),
                    style:
                        t.small.copyWith(color: portColour(t, input.portType)),
                    overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
        if (driven != null)
          Positioned.fill(
            child: Padding(
              padding: const EdgeInsets.only(right: graphRowInset),
              child: Align(alignment: Alignment.centerRight, child: driven),
            ),
          ),
        // Its name, unless a control has the row: the words would sit on top of
        // it, and the socket alone says what the row gives out.
        if (output != null && control == null)
          Positioned.fill(
            child: Padding(
              padding: const EdgeInsets.only(right: graphRowInset),
              child: Align(
                alignment: Alignment.centerRight,
                child: Text(engineLabel(output.label),
                    key: ValueKey<String>(
                        'graph-port-$_key-out-${output.id}'),
                    style:
                        t.small.copyWith(color: portColour(t, output.portType)),
                    overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
        if (input != null)
          Positioned(
            left: -graphSocketSize / 2 - 1,
            top: (row.height - graphSocketSize) / 2,
            child: _socket(t, input, isInput: true),
          ),
        if (output != null)
          Positioned(
            right: -graphSocketSize / 2 - 1,
            top: (row.height - graphSocketSize) / 2,
            child: _socket(t, output, isInput: false),
          ),
      ],
    );
  }

  /// A filled socket is wired, a hollow one is not (15-DESIGN §12A.7). The
  /// one a wire in hand has snapped to is filled and ringed. The colour is the
  /// type, and the tooltip says which.
  Widget _socket(LumitTheme t, BridgePort port, {required bool isInput}) {
    final colour = portColour(t, port.portType);
    final lit = snap?.isInput == isInput && snap?.port.id == port.id;
    return LumitTooltip(
      message: graphPortWord(port.portType),
      child: Container(
        key: ValueKey<String>('graph-socket-$_key-${port.id}'),
        width: graphSocketSize,
        height: graphSocketSize,
        decoration: BoxDecoration(
          color: port.wired || lit ? colour : t.surface1,
          shape: BoxShape.circle,
          border: Border.all(
              color: lit ? t.animated : colour, width: lit ? 2 : 1),
        ),
      ),
    );
  }
}

/// A node's frame, drawn rather than decorated so that a bypassed one can be
/// dashed — Flutter's `Border` has no dash.
/// Public because the inner shader graph's cards wear the identical frame
/// (the canvas is shared, the model is not).
class GraphNodeFrame extends StatelessWidget {
  final Color colour;
  final Color fill;
  final bool dashed;
  final double radius;
  const GraphNodeFrame({
    super.key,
    required this.colour,
    required this.fill,
    required this.dashed,
    required this.radius,
  });

  @override
  Widget build(BuildContext context) => CustomPaint(
        painter: _FramePainter(colour, fill, dashed, radius),
      );
}

class _FramePainter extends CustomPainter {
  final Color colour;
  final Color fill;
  final bool dashed;
  final double radius;
  const _FramePainter(this.colour, this.fill, this.dashed, this.radius);

  @override
  void paint(Canvas canvas, Size size) {
    final rect = RRect.fromRectAndRadius(
      Rect.fromLTWH(0.5, 0.5, size.width - 1, size.height - 1),
      Radius.circular(radius),
    );
    canvas.drawRRect(rect, Paint()..color = fill);
    final stroke = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    if (!dashed) {
      canvas.drawRRect(rect, stroke);
      return;
    }
    canvas.drawPath(graphDashPath(Path()..addRRect(rect)), stroke);
  }

  @override
  bool shouldRepaint(_FramePainter old) =>
      old.colour != colour ||
      old.fill != fill ||
      old.dashed != dashed ||
      old.radius != radius;
}
