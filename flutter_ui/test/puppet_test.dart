
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/viewer_puppet.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

BridgePuppetPin pin(
  BridgePuppetPinKind kind, {
  BridgeScalar? x,
  BridgeScalar? y,
  double extent = 50,
}) =>
    BridgePuppetPin(
      id: UuidValue.fromString(const Uuid().v4()),
      name: 'Pin',
      kind: kind,
      x: x ?? const BridgeScalar.static_(10),
      y: y ?? const BridgeScalar.static_(20),
      rotation: const BridgeScalar.static_(0),
      scale: const BridgeScalar.static_(100),
      amount: const BridgeScalar.static_(0),
      extent: extent,
    );

void main() {
  group('Which pin each tool places', () {
    test('one kind per tool', () {
      expect(puppetKindFor(ToolMode.puppetPosition),
          BridgePuppetPinKind.position);
      expect(puppetKindFor(ToolMode.puppetStarch), BridgePuppetPinKind.starch);
      expect(
          puppetKindFor(ToolMode.puppetOverlap), BridgePuppetPinKind.overlap);
      expect(puppetKindFor(ToolMode.puppetBend), BridgePuppetPinKind.bend);
    });
  });

  group('Writing one of a pin\'s numbers', () {
    test('replaces that one and carries the rest', () {
      final was = pin(BridgePuppetPinKind.bend, extent: 80);
      final now = puppetPinWithScalar(
          was, PuppetValue.rotation, const BridgeScalar.static_(45));
      expect(now.rotation, const BridgeScalar.static_(45));
      expect(now.x, was.x);
      expect(now.y, was.y);
      expect(now.scale, was.scale);
      expect(now.amount, was.amount);
      expect(now.extent, 80);
      expect(now.id, was.id);
      expect(now.kind, was.kind);
    });
  });
}
