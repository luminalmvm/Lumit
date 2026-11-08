// Where a track's sound goes: the master or a bus, and its sends. The rows
// that choose them sit in the Audio timeline's track menu, and the send
// levels on the track's own Sends rows.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart' show BridgeLayerEntry;
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/routes.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// Change one of [layer]'s sends, keeping the rest of its routing. Read at
/// the moment of the write, from a gesture, never from a build.
void writeSend(LayerReference layer, UuidValue bus,
    {BridgeScalar? level, bool? pre}) {
  final info = layer.getInfo();
  layer.setAudioRoutes(
    output: info.output,
    sends: [
      for (final s in info.sends)
        s.bus == bus
            ? BridgeSend(bus: bus, level: level ?? s.level, pre: pre ?? s.pre)
            : s,
    ],
  );
}

/// The track menu's two routing rows: **Output**, the master or one bus, and
/// **Send to**, a tick per bus that adds or takes away a send to it.
List<Widget> audioRouteRows({
  required BridgeLayerEntry entry,
  required List<BridgeLayerEntry> buses,
  required VoidCallback closeParent,
  required VoidCallback onChanged,
}) {
  final info = entry.info;
  Widget tickRow(String key, bool on, String label, VoidCallback act,
          VoidCallback dismiss) =>
      MenuRow(
        key: ValueKey<String>(key),
        onPressed: () {
          dismiss();
          act();
          onChanged();
        },
        child: Row(children: [menuTick(on), Expanded(child: Text(label))]),
      );
  void route({UuidValue? output, required List<BridgeSend> sends}) =>
      entry.layer.setAudioRoutes(output: output, sends: sends);
  return [
    SubmenuRow(
      key: const ValueKey<String>('atl-route-output'),
      closeParent: closeParent,
      submenu: (dismiss) => FloatSurface(
        width: 180,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            tickRow(
                'atl-route-master',
                info.output == null,
                l10n.audioRouteMaster,
                () => route(output: null, sends: info.sends),
                dismiss),
            for (final bus in buses)
              tickRow(
                  'atl-route-output-${bus.layer.internallayerId}',
                  info.output == bus.layer.internallayerId,
                  bus.info.name,
                  () => route(
                      output: bus.layer.internallayerId, sends: info.sends),
                  dismiss),
          ],
        ),
      ),
      child: Text(l10n.audioRouteOutput),
    ),
    if (buses.isNotEmpty)
      SubmenuRow(
        key: const ValueKey<String>('atl-route-send'),
        closeParent: closeParent,
        submenu: (dismiss) => FloatSurface(
          width: 180,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final bus in buses)
                () {
                  final id = bus.layer.internallayerId;
                  final on = info.sends.any((s) => s.bus == id);
                  return tickRow(
                      'atl-route-send-$id',
                      on,
                      bus.info.name,
                      () => route(output: info.output, sends: [
                            for (final s in info.sends)
                              if (s.bus != id) s,
                            if (!on)
                              BridgeSend(
                                  bus: id,
                                  level: const BridgeScalar.static_(0),
                                  pre: false),
                          ]),
                      dismiss);
                }(),
            ],
          ),
        ),
        child: Text(l10n.audioRouteSendTo),
      ),
  ];
}
