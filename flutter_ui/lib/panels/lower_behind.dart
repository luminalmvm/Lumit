// Lower behind: a layer's sound brought down while another layer, or a bus,
// is heard. The popover the Audio panel opens from its button, and the list
// the shared sound menu offers.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'timeline_extras_frb.dart' show iconForKind;

/// What [self] can be lowered behind: the other layers of [layers] that make
/// a sound, and the buses. Each probe opens a file, so this is asked as a menu
/// opens, never in a build.
Future<List<BridgeLayerEntry>> duckSources(
    List<BridgeLayerEntry> layers, UuidValue self) async {
  final out = <BridgeLayerEntry>[];
  for (final entry in layers) {
    if (entry.layer.internallayerId == self) continue;
    try {
      if (entry.info.kind == BridgeLayerKind.bus ||
          await entry.layer.hasAudio()) {
        out.add(entry);
      }
    } catch (_) {
      // The layer has gone.
    }
  }
  return out;
}

/// Write [duck] on [layer], or take it off. A duck that would bring the sound
/// back round to itself is refused, and said so.
bool writeDuck(LumitState app, LayerReference layer, BridgeDuck? duck) {
  try {
    layer.setDuck(duck: duck);
    return true;
  } catch (_) {
    app.postNotice(l10n.audioRouteRefused);
    return false;
  }
}

/// A source's name, with the bus glyph in front of a bus. A layer keeps the
/// glyph's room, so every name starts in one column.
Widget duckSourceLabel(LumitTheme t, BridgeLayerEntry source,
        {required bool ticked}) =>
    Row(children: [
      menuTick(ticked),
      SizedBox(
        width: iconSize,
        child: source.info.kind == BridgeLayerKind.bus
            ? lumitIcon(iconForKind(BridgeLayerKind.bus),
                size: iconSize, color: t.textMuted)
            : null,
      ),
      const SizedBox(width: 4),
      Expanded(
        child: Text(source.info.name,
            maxLines: 1, overflow: TextOverflow.ellipsis),
      ),
    ]);

/// **Lower behind ▸**: the sources, as a flyout. The comp's layers are probed
/// as it opens, and a pick lowers [targets] behind it with the usual settings.
class LowerBehindFlyout extends StatefulWidget {
  final BridgeLayerEntry entry;
  final List<BridgeLayerEntry> targets;
  final VoidCallback dismiss;
  final VoidCallback onChanged;

  const LowerBehindFlyout({
    super.key,
    required this.entry,
    required this.targets,
    required this.dismiss,
    required this.onChanged,
  });

  @override
  State<LowerBehindFlyout> createState() => _LowerBehindFlyoutState();
}

class _LowerBehindFlyoutState extends State<LowerBehindFlyout> {
  List<BridgeLayerEntry>? _sources;

  @override
  void initState() {
    super.initState();
    final layers =
        Provider.of<LumitUiState>(context, listen: false).model.heldLayers;
    duckSources(layers, widget.entry.layer.internallayerId).then((sources) {
      if (mounted) setState(() => _sources = sources);
    });
  }

  void _pick(BridgeLayerEntry source) {
    widget.dismiss();
    final app = Provider.of<LumitState>(context, listen: false);
    final id = source.layer.internallayerId;
    var refused = false;
    asOneUndoStep(app.project, () {
      for (final t in widget.targets) {
        if (t.layer.internallayerId == id) continue;
        try {
          t.layer.setDuck(duck: duckBehind(behind: id));
        } catch (_) {
          // A locked layer refuses quietly, and the rest still take it.
          if (!t.info.switches.locked) refused = true;
        }
      }
    });
    if (refused) app.postNotice(l10n.audioRouteRefused);
    widget.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final sources = _sources;
    final current = widget.entry.info.duck?.behind;
    return FloatSurface(
      width: 200,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 380),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (sources != null && sources.isEmpty)
                Padding(
                  padding: const EdgeInsets.all(8),
                  child: Text(l10n.audioDuckNoSources, style: t.small),
                ),
              for (final source in sources ?? const <BridgeLayerEntry>[])
                MenuRow(
                  key: ValueKey<String>(
                      'audio-menu-duck-${source.layer.internallayerId}'),
                  onPressed: () => _pick(source),
                  child: duckSourceLabel(t, source,
                      ticked: current == source.layer.internallayerId),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Open the Lower behind popover under [anchor]'s own box: what [entry] is
/// lowered behind, by how much, how loud the source must be, how quickly it
/// goes down and comes back, and Remove. [sources] is what it can be lowered
/// behind.
Future<void> showLowerBehind({
  required BuildContext anchor,
  required BridgeLayerEntry entry,
  required List<BridgeLayerEntry> sources,
  required VoidCallback onChanged,
}) {
  final box = anchor.findRenderObject() as RenderBox?;
  final at = box == null
      ? Offset.zero
      : box.localToGlobal(box.size.bottomLeft(Offset.zero));
  return showLumitPopup<void>(
    context: anchor,
    position: at,
    builder: (close) => _LowerBehindPopover(
      entry: entry,
      sources: sources,
      close: () => close(null),
      onChanged: onChanged,
    ),
  );
}

class _LowerBehindPopover extends StatefulWidget {
  final BridgeLayerEntry entry;
  final List<BridgeLayerEntry> sources;
  final VoidCallback close;
  final VoidCallback onChanged;

  const _LowerBehindPopover({
    required this.entry,
    required this.sources,
    required this.close,
    required this.onChanged,
  });

  @override
  State<_LowerBehindPopover> createState() => _LowerBehindPopoverState();
}

class _LowerBehindPopoverState extends State<_LowerBehindPopover> {
  /// The duck as written, held here since the panel behind does not rebuild
  /// this popover.
  late BridgeDuck? _duck = widget.entry.info.duck;

  /// The value a drag has in hand, by row, until it is let go.
  final Map<String, double> _staged = {};

  void _write(BridgeDuck? next) {
    final app = Provider.of<LumitState>(context, listen: false);
    if (writeDuck(app, widget.entry.layer, next)) {
      if (mounted) setState(() => _duck = next);
      widget.onChanged();
    }
  }

  BridgeDuck _with(BridgeDuck d, String row, double v) => BridgeDuck(
        behind: d.behind,
        depthDb: row == 'amount' ? v : d.depthDb,
        thresholdDb: row == 'threshold' ? v : d.thresholdDb,
        attackMs: row == 'attack' ? v : d.attackMs,
        holdMs: d.holdMs,
        releaseMs: row == 'release' ? v : d.releaseMs,
      );

  Widget _row(LumitTheme t, BridgeDuck d, String row, String label,
      double value,
      {required double min,
      required double max,
      required String suffix,
      required int decimals}) {
    void commit(num v) {
      _staged.remove(row);
      _write(_with(d, row, v.toDouble()));
    }

    return Padding(
      padding: const EdgeInsets.only(top: 6),
      child: Row(children: [
        Expanded(
            child: Text(label, style: t.body.copyWith(color: t.textMuted))),
        SizedBox(
          width: 76,
          child: DragValueField(
            key: ValueKey<String>('audio-duck-$row'),
            value: _staged[row] ?? value,
            min: min,
            max: max,
            decimals: decimals,
            suffix: suffix,
            speed: decimals == 0 ? 1 : 0.2,
            onChanged: commit,
            onChangeLive: (v) => setState(() => _staged[row] = v.toDouble()),
            onChangeEnd: commit,
            onDragCancel: () => setState(() => _staged.remove(row)),
          ),
        ),
      ]),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final d = _duck;
    return FloatSurface(
      width: 220,
      child: Padding(
        padding: const EdgeInsets.only(bottom: 8),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 200),
              child: SingleChildScrollView(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    if (widget.sources.isEmpty)
                      Padding(
                        padding: const EdgeInsets.all(8),
                        child: Text(l10n.audioDuckNoSources, style: t.small),
                      ),
                    for (final source in widget.sources)
                      MenuRow(
                        key: ValueKey<String>(
                            'audio-duck-source-${source.layer.internallayerId}'),
                        onPressed: () {
                          final id = source.layer.internallayerId;
                          if (d?.behind == id) return;
                          _write(d == null
                              ? duckBehind(behind: id)
                              : BridgeDuck(
                                  behind: id,
                                  depthDb: d.depthDb,
                                  thresholdDb: d.thresholdDb,
                                  attackMs: d.attackMs,
                                  holdMs: d.holdMs,
                                  releaseMs: d.releaseMs,
                                ));
                        },
                        child: duckSourceLabel(t, source,
                            ticked: d?.behind == source.layer.internallayerId),
                      ),
                  ],
                ),
              ),
            ),
            if (d != null)
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 2, 10, 0),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    const MenuDivider(),
                    _row(t, d, 'amount', l10n.duckAmount, d.depthDb,
                        min: -60, max: 0, suffix: ' dB', decimals: 1),
                    _row(t, d, 'threshold', l10n.duckThreshold, d.thresholdDb,
                        min: -60, max: 0, suffix: ' dB', decimals: 1),
                    _row(t, d, 'attack', l10n.duckAttack, d.attackMs,
                        min: 1, max: 500, suffix: ' ms', decimals: 0),
                    _row(t, d, 'release', l10n.duckRelease, d.releaseMs,
                        min: 10, max: 5000, suffix: ' ms', decimals: 0),
                    const SizedBox(height: 8),
                    Align(
                      alignment: Alignment.centerRight,
                      child: HouseButton(
                        key: const ValueKey<String>('audio-duck-remove'),
                        small: true,
                        onPressed: () {
                          _write(null);
                          widget.close();
                        },
                        child: Text(l10n.duckRemove),
                      ),
                    ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }
}
