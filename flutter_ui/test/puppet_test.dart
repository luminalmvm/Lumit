// The Puppet tools' own arithmetic (PU3 — test 15 of
// docs/impl/puppet.md's plan, the half that needs no engine).
//
// Three things are easy to get subtly wrong and are all here: which kind of pin
// each tool places, which of a pin's numbers a pin of that kind actually shows
// in the Timeline (a position pin has no amount, and only a bend pin turns), and
// that an edit to one of those numbers leaves the other five exactly as they
// were. The round trip through the document, the undo and the two refusals live
// where the document does, in `lumit_bridge`'s own tests.

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
  group('The four tools are armable', () {
    test('every puppet tool can be armed, by click and by chord', () {
      final members = ToolMode.builtMembersOf(ToolGroup.puppet);
      expect(members, ToolMode.membersOf(ToolGroup.puppet),
          reason: 'all four are built, so none is drawn disabled');

      final tools = ToolsState();
      for (final tool in members) {
        tools.select(tool);
        expect(tools.tool, tool);
      }
    });

    test('the chord walks all four and wraps', () {
      final tools = ToolsState()..cycleGroup(ToolGroup.puppet);
      final walked = <ToolMode>[tools.tool];
      for (var i = 0; i < 4; i++) {
        tools.cycleGroup(ToolGroup.puppet);
        walked.add(tools.tool);
      }
      expect(walked.first, walked.last, reason: 'five presses of four wraps');
      expect(walked.toSet().length, 4);
    });
  });

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

  group('The rows a pin grows in the Timeline', () {
    test('a position pin has only a place', () {
      expect(puppetValuesFor(BridgePuppetPinKind.position),
          [PuppetValue.positionX, PuppetValue.positionY]);
    });

    test('starch and overlap add how much, and never a turn', () {
      for (final kind in [
        BridgePuppetPinKind.starch,
        BridgePuppetPinKind.overlap
      ]) {
        final values = puppetValuesFor(kind);
        expect(values, contains(PuppetValue.amount));
        expect(values, isNot(contains(PuppetValue.rotation)));
        expect(values, isNot(contains(PuppetValue.scale)));
      }
    });

    test('a bend pin turns and scales, and has no amount', () {
      final values = puppetValuesFor(BridgePuppetPinKind.bend);
      expect(values, contains(PuppetValue.rotation));
      expect(values, contains(PuppetValue.scale));
      expect(values, isNot(contains(PuppetValue.amount)));
    });

    test('the extent is on no lane: it is not animatable', () {
      for (final kind in BridgePuppetPinKind.values) {
        expect(puppetValuesFor(kind).length, lessThanOrEqualTo(4));
      }
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

    test('reads back through the same map', () {
      final was = pin(BridgePuppetPinKind.starch);
      expect(puppetScalarOf(was, PuppetValue.positionX), was.x);
      expect(puppetScalarOf(was, PuppetValue.amount), was.amount);
    });
  });

  group('Where the overlay draws a pin', () {
    test('a still pin is where it says it is', () {
      expect(puppetPinAt(pin(BridgePuppetPinKind.position)),
          const Offset(10, 20));
    });

    test('a keyed pin falls back to its first key rather than to the origin',
        () {
      final keyed = pin(
        BridgePuppetPinKind.position,
        x: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: const BridgeRational(num: 0, den: 1),
            value: 33,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ]),
      );
      expect(puppetPinAt(keyed).dx, 33);
    });
  });

  group('The mesh options are session state, clamped', () {
    test('density and expansion are held inside what the engine will build', () {
      final tools = ToolsState();
      expect(tools.puppetDensity, 24);
      expect(tools.puppetExpansion, 3);

      tools.puppetDensity = 0;
      expect(tools.puppetDensity, 2, reason: 'a mesh of nothing is not a mesh');
      tools.puppetExpansion = -5;
      expect(tools.puppetExpansion, 0);
    });
  });
}
