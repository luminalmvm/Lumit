// A driver wired onto a parameter as one undo step, from wherever the row is.
//
// The Audio panel's two templates and the parameter row menu's Drive with
// stage the same thing: new driver boxes, their wires and one setGraph, so the
// Graph panel shows exactly what was built and one undo takes it all away.
// A socket that already had a wire is re-routed rather than doubled, the rule
// a hand-drawn wire follows.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import 'shader_editor.dart' show InstanceHome, editExpressionOn;

/// What a row's menu can wire onto it.
enum DriveKind { wiggle, audioLevel, beatTrigger, colourCycle, expression }

/// The output feeding [target], or null while nothing is wired into it.
BridgeOutputRef? wiredInto(BridgeLayerGraph graph, BridgeInputRef target) {
  for (final e in graph.wiring.edges) {
    if (e.to == target) return e.from;
  }
  return null;
}

/// The box on a layer's graph whose instance id is [id]: an effect's or a
/// driver's. Null for an id the graph has no box for, which is a clip's or a
/// group's effect.
BridgeNodeRef? graphNodeWithId(BridgeLayerGraph graph, UuidValue id) {
  for (final n in graph.nodes) {
    switch (n.node) {
      case BridgeNodeRef_Effect(:final field0) when field0 == id:
      case BridgeNodeRef_Driver(:final field0) when field0 == id:
        return n.node;
      default:
        break;
    }
  }
  return null;
}

/// Audio level's and Beat trigger's Source row, the entry that reads one named
/// layer. A number because a Choice row crosses the bridge as its index.
const int _sourceLayer = 1;

/// The chain for [kind], wired into [target], committed as one step. False
/// when the engine refused any part of it, and nothing is written then.
///
/// [current] is the number the row shows and [low] and [high] its range, so a
/// Wiggle wobbles about the value that was there and a sound lifts it from
/// there to the top of its range. [listenTo] names the layer Audio level or
/// Beat trigger listens to; null is the whole composition. [outLow] and
/// [outHigh] override the audio chain's range where the caller knows better.
bool stageDrive({
  required LayerReference layer,
  required BridgeInputRef target,
  required DriveKind kind,
  double current = 0,
  double low = 0,
  double high = 100,
  UuidValue? listenTo,
  double? outLow,
  double? outHigh,
  bool colour = false,
}) {
  final BridgeLayerGraph graph;
  final drivers = <BridgeEffectInstance>[];
  final edges = <BridgeGraphEdge>[];
  BridgeEffectInstance mint(String name) {
    final d = layer.newDriver(name: name);
    drivers.add(d);
    return d;
  }

  BridgeOutputRef out(BridgeEffectInstance from, String port) =>
      BridgeOutputRef.driver(node: from.id(), port: port);
  BridgeInputRef into(BridgeEffectInstance to, String port) =>
      BridgeInputRef.param(node: BridgeNodeRef.driver(to.id()), port: port);
  BridgeEffectValue number(double v) =>
      BridgeEffectValue.float(BridgeScalar.static_(v));
  final span = (high - low).abs() < 1e-6 ? 100.0 : (high - low);
  try {
    graph = layer.getGraph();
    switch (kind) {
      case DriveKind.wiggle:
        // Wiggle strays either side of nought, so Math adds the value that was
        // there: the row wobbles about its number rather than about zero.
        final wiggle = mint('wiggle');
        wiggle.setValue(id: 'amount', value: number(span / 10));
        final add = mint('math');
        add.setValue(id: 'operation', value: const BridgeEffectValue.choice(0));
        add.setValue(id: 'b', value: number(current));
        edges
          ..add(BridgeGraphEdge(from: out(wiggle, 'value'), to: into(add, 'a')))
          ..add(BridgeGraphEdge(from: out(add, 'value'), to: target));
      case DriveKind.audioLevel:
      case DriveKind.beatTrigger:
        final ears = mint(kind == DriveKind.audioLevel ? 'audio_level' : 'beat_trigger');
        if (listenTo != null) {
          ears.setValue(id: 'audio', value: BridgeEffectValue.layer(listenTo));
          ears.setValue(
              id: 'source', value: const BridgeEffectValue.choice(_sourceLayer));
        }
        final remap = mint('remap');
        if (kind == DriveKind.audioLevel) {
          // A practical loudness window: RMS of finished music rarely clears
          // 0.3, so the chain reaches its far end on real material rather
          // than only on full scale.
          remap.setValue(id: 'in_high', value: number(0.3));
        }
        remap.setValue(id: 'out_low', value: number(outLow ?? current));
        remap.setValue(id: 'out_high', value: number(outHigh ?? high));
        var last = remap;
        var fromPort = 'value';
        edges.add(BridgeGraphEdge(
            from: out(ears, kind == DriveKind.audioLevel ? 'amplitude' : 'envelope'),
            to: into(remap, 'value')));
        if (kind == DriveKind.audioLevel) {
          final smooth = mint('smooth');
          edges.add(BridgeGraphEdge(from: out(remap, 'value'), to: into(smooth, 'value')));
          last = smooth;
          fromPort = 'value';
        }
        edges.add(BridgeGraphEdge(from: out(last, fromPort), to: target));
      case DriveKind.colourCycle:
        final cycle = mint('colour_cycle');
        edges.add(BridgeGraphEdge(from: out(cycle, 'colour'), to: target));
      case DriveKind.expression:
        final expression = mint('expression');
        edges.add(BridgeGraphEdge(
            from: out(expression, colour ? 'colour' : 'value'), to: target));
    }
    final kept = [...graph.wiring.edges]..removeWhere((e) => e.to == target);
    layer.setGraph(
      drivers: [...layer.getGraphDrivers(), ...drivers],
      wiring: BridgeGraphWiring(
        edges: [...kept, ...edges],
        // No positions for the new boxes: the Graph panel places what has no
        // entry, as it does for a fresh layer.
        layout: graph.wiring.layout,
        exposed: graph.wiring.exposed,
        groups: graph.wiring.groups,
        outUnwired: graph.wiring.outUnwired,
      ),
    );
  } catch (_) {
    return false;
  }
  return true;
}

/// The Audio panel's templates: Audio level, Remap and Smooth into [target],
/// listening to [listenTo] or the whole composition. [inverted] is the duck,
/// silence in to 0 dB out and loud in to a bed 18 dB under.
bool stageAudioChain({
  required LayerReference layer,
  required BridgeInputRef target,
  required bool inverted,
  UuidValue? listenTo,
}) =>
    stageDrive(
      layer: layer,
      target: target,
      kind: DriveKind.audioLevel,
      listenTo: listenTo,
      outLow: 0,
      outHigh: inverted ? -18 : 100,
    );

/// Take the wire off [target], and with it every driver that fed only that
/// wire, so the chain Drive with built goes as one step. A driver still
/// feeding something else stays. False when the engine refused.
bool removeDriveFrom(LayerReference layer, BridgeInputRef target) {
  try {
    final graph = layer.getGraph();
    final edges = [...graph.wiring.edges];
    final gone = <UuidValue>{};
    // The sources to look at: a driver that has just lost its last wire.
    final pending = <UuidValue>[];
    void unwire(BridgeInputRef to) {
      for (final e in edges.where((e) => e.to == to).toList()) {
        edges.remove(e);
        if (e.from case BridgeOutputRef_Driver(:final node)) pending.add(node);
      }
    }

    unwire(target);
    while (pending.isNotEmpty) {
      final id = pending.removeLast();
      if (gone.contains(id)) continue;
      if (edges.any((e) => _leaves(e.from, id))) continue;
      gone.add(id);
      final node = BridgeNodeRef.driver(id);
      for (final e in edges.where((e) => _lands(e.to, node)).toList()) {
        edges.remove(e);
        if (e.from case BridgeOutputRef_Driver(node: final from)) {
          pending.add(from);
        }
      }
    }
    bool kept(BridgeNodeRef n) =>
        n is! BridgeNodeRef_Driver || !gone.contains(n.field0);
    layer.setGraph(
      drivers: [
        for (final d in layer.getGraphDrivers())
          if (!gone.contains(d.id())) d,
      ],
      wiring: BridgeGraphWiring(
        edges: edges,
        layout: [for (final p in graph.wiring.layout) if (kept(p.node)) p],
        exposed: [for (final n in graph.wiring.exposed) if (kept(n)) n],
        groups: [
          for (final g in graph.wiring.groups)
            if (g.members.any(kept))
              BridgeNodeGroup(
                name: g.name,
                colour: g.colour,
                members: [for (final m in g.members) if (kept(m)) m],
              ),
        ],
        outUnwired: graph.wiring.outUnwired,
      ),
    );
  } catch (_) {
    return false;
  }
  return true;
}

bool _lands(BridgeInputRef to, BridgeNodeRef on) => switch (to) {
      BridgeInputRef_Param(:final node) => node == on,
      _ => false,
    };

bool _leaves(BridgeOutputRef from, UuidValue driver) => switch (from) {
      BridgeOutputRef_Driver(:final node) => node == driver,
      _ => false,
    };

/// A row's range, for how far a drive moves it: a slider's own, a number's
/// soft one, and a turn for an angle.
(double, double) driveRange(BridgeParamKind kind) => switch (kind) {
      BridgeParamKind_Slider(:final min, :final max) => (min, max),
      BridgeParamKind_Float(:final sliderMin, :final sliderMax) => (
          sliderMin,
          sliderMax
        ),
      BridgeParamKind_Int(:final sliderMin, :final sliderMax) => (
          sliderMin.toDouble(),
          sliderMax.toDouble()
        ),
      BridgeParamKind_Angle() => (0, 360),
      _ => (0, 100),
    };

/// Wiggle as an expression, for a row with no socket: a transform's. [axis]
/// offsets the noise so a pair's two numbers stray apart.
String wiggleExpression(BridgeExpressionLanguage language, double current,
        double amount, int axis) =>
    switch (language) {
      BridgeExpressionLanguage.javaScript => 'wiggle(2, ${_short(amount)})',
      BridgeExpressionLanguage.rhai =>
        '${_short(current)} + (noise(time * 2 + ${axis * 100}) * 2 - 1) * ${_short(amount)}',
    };

String _short(double v) => double.parse(v.toStringAsFixed(3)).toString();

/// Open the editor of the Expression driver feeding [target], the step after
/// Drive with ▸ Expression: the box is picked on the graph so the Node panel
/// shows it, and the editor comes up over the panel.
void editDriveExpression(
    BuildContext context, LayerReference layer, BridgeInputRef target) {
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final BridgeOutputRef? from;
  try {
    from = wiredInto(layer.getGraph(), target);
  } catch (_) {
    return;
  }
  if (from case BridgeOutputRef_Driver(:final node)) {
    ui.graphNode.value = BridgeNodeRef.driver(node);
    editExpressionOn(
      context: context,
      home: InstanceHome.drivers(layer),
      effect: node,
      param: 'edit',
      onApplied: ui.model.refresh,
    );
  }
}

/// The one notice a refused chain gets. Which part refused is not something
/// the user can act on differently, so they share a sentence.
void sayDriveRefused(BuildContext context) =>
    Provider.of<LumitState>(context, listen: false)
        .postNotice(l10n.driveNotBuilt, error: true);
