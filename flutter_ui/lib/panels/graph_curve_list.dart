// The curves on the graph, named, in the corner of the plot: each row wears its
// curve's colour, a click puts the curve away or brings it back, and a right
// click is the curve's menu. There is no list while there is one curve to name.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_channels.dart';

/// How wide the list may grow before a name is cut.
const double _listWidth = 280;
const double _rowHeight = 18;

/// How wide the property's name is with a layer's name beside it, so the
/// layers' names share one edge.
const double _nameWidth = 96;

/// A row's property name: all the room when it is alone, a column of its own
/// when a layer's name follows it.
Widget _name(bool alone, Widget text) =>
    alone ? Flexible(child: text) : SizedBox(width: _nameWidth, child: text);

class GraphCurveList extends StatelessWidget {
  final List<GraphChannel> channels;

  /// Whether [channel] is drawn dashed: on show without being picked.
  final bool Function(GraphChannel channel) dashed;
  final String? lit;
  final ValueChanged<String?> onHover;
  final ValueChanged<GraphChannel> onToggle;
  final ValueChanged<GraphChannel> onSolo;
  final void Function(GraphChannel channel, Offset at) onMenu;
  final double maxHeight;

  const GraphCurveList({
    super.key,
    required this.channels,
    required this.dashed,
    required this.lit,
    required this.onHover,
    required this.onToggle,
    required this.onSolo,
    required this.onMenu,
    required this.maxHeight,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final hidden = hiddenCurves.value;
    // What differs comes first, so a long layer name can't push every row's
    // property off the end. One layer has no need to say its name at all.
    final oneLayer = {
          for (final c in channels) c.entry.layer.internallayerId,
        }.length ==
        1;

    return ConstrainedBox(
      constraints: BoxConstraints(maxWidth: _listWidth, maxHeight: maxHeight),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: t.surface0,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        ),
        child: SingleChildScrollView(
          padding: const EdgeInsets.symmetric(vertical: 3),
          child: IntrinsicWidth(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final c in channels)
                  _row(t, c, oneLayer, hidden.contains(c.id)),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _row(LumitTheme t, GraphChannel c, bool oneLayer, bool off) {
    final colour = t.curve[c.colourIndex % t.curve.length];
    final lead = '${c.entry.info.name} · ';
    final name =
        c.label.startsWith(lead) ? c.label.substring(lead.length) : c.label;
    final bright = lit == c.id;
    return MouseRegion(
      onEnter: (_) => onHover(c.id),
      onExit: (_) => onHover(null),
      child: GestureDetector(
        key: ValueKey<String>('graph-curve-${c.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: () =>
            HardwareKeyboard.instance.isAltPressed ? onSolo(c) : onToggle(c),
        onSecondaryTapDown: (d) => onMenu(c, d.globalPosition),
        child: SizedBox(
          height: _rowHeight,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 6),
            child: Row(
              children: [
                // The curve's own stroke, or its outline once it is put away.
                Container(
                  width: 10,
                  height: off ? 1 : 2,
                  color: off ? t.hairlineStrong : colour,
                ),
                const SizedBox(width: 6),
                _name(
                    oneLayer,
                    Text(
                      name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: t.small.copyWith(
                        color: off
                            ? t.textDisabled
                            : bright
                                ? t.textPrimary
                                : dashed(c)
                                    ? t.textMuted
                                    : t.textSecondary,
                      ),
                    )),
                if (!oneLayer) ...[
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      c.entry.info.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: t.small
                          .copyWith(color: off ? t.textDisabled : t.textMuted),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
