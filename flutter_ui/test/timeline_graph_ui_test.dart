// Section-C timeline/graph UI tests: the keyframe interpolation right-click
// menu (Easy ease / Linear / Hold / Unify / Delete), the empty-lane context menu
// (composition settings / reveal / grid / beats), the comp-strip pop-out notice,
// the keyframe copy/paste clipboard, and the transport work-area loop. Pure logic
// is unit-tested; the menus are driven through a mounted overlay with a recording
// fake bridge.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/timeline/keyframe_clipboard.dart';
import 'package:lumit_flutter/panels/timeline/keyframe_interp_menu.dart';
import 'package:lumit_flutter/panels/timeline/lane_context_menu.dart';
import 'package:lumit_flutter/panels/timeline/lane_selection.dart';
import 'package:lumit_flutter/panels/timeline_panel.dart';
import 'package:lumit_flutter/panels/viewer_panel.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// A comp with one footage layer whose `position_x` is animated with two keys —
/// a plain linear key at frame 60 and a broken-bezier key at frame 120.
const _compJson = '''
{
  "ok": true,
  "items": [
    {
      "id": "c0", "name": "Scene", "kind": "composition", "children": [],
      "comp": {
        "width": 1920, "height": 1080,
        "fps": {"num": 60, "den": 1}, "frame_count": 300,
        "layers": [
          {
            "id": "l0", "index": 0, "name": "hero", "kind": "footage",
            "in_frame": 0, "out_frame": 300, "label": 2,
            "switches": {"visible": true, "audible": true, "locked": false,
              "three_d": false, "collapse": false, "fx": true,
              "solo": false, "motion_blur": false},
            "transform": {
              "position_x": {
                "value": 0, "animated": true,
                "keys": [
                  {"frame": 60, "value": 100, "interp_in": "Linear",
                   "interp_out": "Linear"},
                  {"frame": 120, "value": 200, "interp_in": "Bezier",
                   "interp_out": "Bezier",
                   "bezier_in": {"speed": 2, "influence": 0.25},
                   "bezier_out": {"speed": 6, "influence": 0.4}}
                ]
              }
            }
          }
        ],
        "markers": []
      }
    }
  ],
  "can_undo": false, "can_redo": false
}
''';

BridgeComp _comp() =>
    BridgeReply.parse(_compJson).snapshot!.items.first.comp!;

BridgeKeyframe _keyAt(int frame) {
  final keys = _comp().layers.first.transform!['position_x']!.keys;
  return keys.firstWhere((k) => k.frame == frame);
}

/// A fake offering both bridge surfaces; every op records a line and returns a
/// benign ok snapshot (built from [_compJson] so comp/layer resolution works).
class _RecordFake implements DocumentBridge, EditOpsBridge {
  final List<String> ops = [];

  BridgeSnapshot _snap() => BridgeReply.parse(_compJson).snapshot!;

  BridgeReply _ok(String record) {
    ops.add(record);
    return BridgeReply.ok(_snap());
  }

  @override
  BridgeReply snapshot() => BridgeReply.ok(_snap());

  @override
  BridgeReply setKeyframeInterp(
          String c,
          String l,
          String p,
          int f,
          String ii,
          String io,
          double si,
          double ifl,
          double so,
          double ofl) =>
      _ok('interp:$c/$l/$p@$f:$ii/$io:si=$si,ii=$ifl,so=$so,oi=$ofl');

  @override
  BridgeReply removeKeyframe(String c, String l, String p, int f) =>
      _ok('remove:$c/$l/$p@$f');

  @override
  BridgeReply applyKeyframeBatch(String c, String l, String j) =>
      _ok('batch:$c/$l:$j');

  @override
  BridgeReply detectBeats(String c, int s) => _ok('beats:$c:$s');

  @override
  BridgeReply clearBeatMarkers(String c) => _ok('clear_beats:$c');

  @override
  dynamic noSuchMethod(Invocation invocation) => BridgeReply.ok(_snap());
}

/// Mount [child] under a ThemeScope + Overlay so popups have somewhere to go.
Widget _host(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: MediaQuery(
        data: const MediaQueryData(),
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [OverlayEntry(builder: (_) => child)],
          ),
        ),
      ),
    );

/// A single keyed button that opens a menu via [onOpen] when tapped — the
/// trigger the menu widget tests drive.
Widget _trigger(void Function(BuildContext) onOpen) => Builder(
      builder: (context) => GestureDetector(
        key: const Key('menu-trigger'),
        behavior: HitTestBehavior.opaque,
        onTap: () => onOpen(context),
        child: const SizedBox(width: 200, height: 40),
      ),
    );

void main() {
  group('KeyframeInterpSides (egui graph key menu, graph.rs:1676)', () {
    test('easy ease is Bezier/Bezier, speed 0, influence a third', () {
      const s = KeyframeInterpSides.easyEase;
      expect(s.interpIn, 'Bezier');
      expect(s.interpOut, 'Bezier');
      expect(s.speedIn, 0);
      expect(s.influenceIn, closeTo(1 / 3, 1e-12));
      expect(s.influenceOut, closeTo(1 / 3, 1e-12));
    });

    test('linear and hold set both sides', () {
      expect(KeyframeInterpSides.linear.interpIn, 'Linear');
      expect(KeyframeInterpSides.linear.interpOut, 'Linear');
      expect(KeyframeInterpSides.hold.interpIn, 'Hold');
      expect(KeyframeInterpSides.hold.interpOut, 'Hold');
    });

    test('unify averages the two slopes, keeps each reach', () {
      final s = KeyframeInterpSides.unify(_keyAt(120));
      expect(s.interpIn, 'Bezier');
      expect(s.speedIn, closeTo(4, 1e-12)); // (2 + 6) / 2
      expect(s.speedOut, closeTo(4, 1e-12));
      expect(s.influenceIn, closeTo(0.25, 1e-12)); // in-side reach preserved
      expect(s.influenceOut, closeTo(0.4, 1e-12)); // out-side reach preserved
    });

    test('unify is offered only for a broken bezier key', () {
      expect(unifyEligible(_keyAt(120)), isTrue); // 2 != 6
      expect(unifyEligible(_keyAt(60)), isFalse); // linear, no handles
    });
  });

  group('removeBatchJson (applyKeyframeBatch vocabulary)', () {
    test('emits a remove op per key on the layer', () {
      final json = removeBatchJson('l0', {
        LaneKeyId('l0', 'position_x', 60),
        LaneKeyId('l0', 'position_x', 120),
        LaneKeyId('l9', 'opacity', 5), // other layer — excluded
      });
      expect(json, contains('"action":"remove"'));
      expect(json, contains('"frame":60'));
      expect(json, contains('"frame":120'));
      expect(json, isNot(contains('opacity')));
    });
  });

  group('applyKeyframeInterpChoice routing', () {
    late _RecordFake fake;
    late AppStateStub app;
    setUp(() {
      fake = _RecordFake();
      app = AppStateStub(bridge: fake);
    });

    test('easy ease applies to every target via setKeyframeInterp', () {
      applyKeyframeInterpChoice(
        app: app,
        compId: 'c0',
        choice: KeyframeInterpChoice.easyEase,
        hit: _keyAt(60),
        hitId: const LaneKeyId('l0', 'position_x', 60),
        targets: {
          LaneKeyId('l0', 'position_x', 60),
          LaneKeyId('l0', 'position_x', 120),
        },
      );
      final interp = fake.ops.where((o) => o.startsWith('interp:')).toList();
      expect(interp.length, 2);
      expect(interp.every((o) => o.contains('Bezier/Bezier')), isTrue);
    });

    test('unify applies to the hit key alone', () {
      applyKeyframeInterpChoice(
        app: app,
        compId: 'c0',
        choice: KeyframeInterpChoice.unify,
        hit: _keyAt(120),
        hitId: const LaneKeyId('l0', 'position_x', 120),
        targets: {
          LaneKeyId('l0', 'position_x', 60),
          LaneKeyId('l0', 'position_x', 120),
        },
      );
      final interp = fake.ops.where((o) => o.startsWith('interp:')).toList();
      expect(interp.length, 1);
      expect(interp.single, contains('@120'));
      expect(interp.single, contains('si=4.0'));
    });

    test('single delete removes; multi delete batches', () {
      applyKeyframeInterpChoice(
        app: app,
        compId: 'c0',
        choice: KeyframeInterpChoice.delete,
        hit: _keyAt(60),
        hitId: const LaneKeyId('l0', 'position_x', 60),
        targets: {LaneKeyId('l0', 'position_x', 60)},
      );
      expect(fake.ops.any((o) => o.startsWith('remove:')), isTrue);

      fake.ops.clear();
      applyKeyframeInterpChoice(
        app: app,
        compId: 'c0',
        choice: KeyframeInterpChoice.delete,
        hit: _keyAt(60),
        hitId: const LaneKeyId('l0', 'position_x', 60),
        targets: {
          LaneKeyId('l0', 'position_x', 60),
          LaneKeyId('l0', 'position_x', 120),
        },
      );
      expect(fake.ops.any((o) => o.startsWith('batch:')), isTrue);
      expect(fake.ops.any((o) => o.startsWith('remove:')), isFalse);
    });
  });

  group('keyframe clipboard (copy/paste, egui note 2.2)', () {
    test('build anchors offsets on the earliest key and captures easing', () {
      final clip = buildKeyframeClipboard({
        LaneKeyId('l0', 'position_x', 60),
        LaneKeyId('l0', 'position_x', 120),
      }, _comp());
      expect(clip.keys.length, 2);
      final byFrame = {for (final k in clip.keys) k.frameOffset: k};
      expect(byFrame.keys, containsAll(<int>[0, 60]));
      expect(byFrame[0]!.value, 100);
      expect(byFrame[60]!.interpIn, 'Bezier');
      expect(byFrame[60]!.eases, isTrue);
      expect(byFrame[0]!.eases, isFalse);
    });

    test('encode/decode round-trips', () {
      final clip = buildKeyframeClipboard({
        LaneKeyId('l0', 'position_x', 60),
        LaneKeyId('l0', 'position_x', 120),
      }, _comp());
      final back = KeyframeClipboard.decode(clip.encode());
      expect(back.keys.length, clip.keys.length);
      expect(back.layerIds, {'l0'});
    });

    test('decode of null/garbage is an empty clipboard, not a throw', () {
      expect(KeyframeClipboard.decode(null).isEmpty, isTrue);
      expect(KeyframeClipboard.decode('not json').isEmpty, isTrue);
    });

    test('paste anchors add ops and the new selection at the playhead', () {
      final clip = buildKeyframeClipboard({
        LaneKeyId('l0', 'position_x', 60),
        LaneKeyId('l0', 'position_x', 120),
      }, _comp());
      final json = pasteAddBatchJson(clip, 'l0', 200);
      expect(json, contains('"action":"add"'));
      expect(json, contains('"frame":200')); // earliest → playhead
      expect(json, contains('"frame":260')); // +60 offset
      final ids = pastedKeyIds(clip, 200).map((k) => k.frame).toSet();
      expect(ids, {200, 260});
    });
  });

  group('workAreaLoopFrame (transport, playback.rs comp_cached_tick)', () {
    test('loops the whole comp when there is no work area', () {
      expect(
          workAreaLoopFrame(
              current: 299, advance: 1, frameCount: 300, workArea: null),
          0);
      expect(
          workAreaLoopFrame(
              current: 100, advance: 1, frameCount: 300, workArea: null),
          101);
    });

    test('loops the work area [in, out) when one is set', () {
      // out edge is exclusive: reaching 180 wraps back to 30.
      expect(
          workAreaLoopFrame(
              current: 179, advance: 1, frameCount: 300, workArea: [30, 180]),
          30);
      expect(
          workAreaLoopFrame(
              current: 100, advance: 1, frameCount: 300, workArea: [30, 180]),
          101);
    });

    test('a playhead outside the area snaps back to its start', () {
      expect(
          workAreaLoopFrame(
              current: 5, advance: 1, frameCount: 300, workArea: [30, 180]),
          30);
      expect(
          workAreaLoopFrame(
              current: 250, advance: 1, frameCount: 300, workArea: [30, 180]),
          30);
    });

    test('a large advance still lands inside the span (modular wrap)', () {
      // span 150; from 170, +200 → 30 + (170+200-30) % 150 = 30 + 340%150 = 70.
      expect(
          workAreaLoopFrame(
              current: 170, advance: 200, frameCount: 300, workArea: [30, 180]),
          70);
    });
  });

  group('empty-lane context menu (panel.rs:384)', () {
    testWidgets('reveal selects the comp item; detect/clear route to the bridge',
        (tester) async {
      final fake = _RecordFake();
      final app = AppStateStub(bridge: fake);
      app.frontCompSelect('c0');
      await tester.pumpWidget(_host(_trigger((context) {
        showLaneContextMenu(
          context: context,
          app: app,
          compId: 'c0',
          showTimeGrid: false,
          onToggleGrid: () {},
          onCompositionSettings: () async {},
          position: const Offset(20, 20),
        );
      })));
      await tester.tap(find.byKey(const Key('menu-trigger')));
      await tester.pump();
      expect(find.text('Reveal in project'), findsOneWidget);
      expect(find.text('Detect beats'), findsOneWidget);

      await tester.tap(find.text('Reveal in project'));
      await tester.pump();
      expect(app.selectedProjectItem, 'c0');
    });

    testWidgets('detect beats passes the sensitivity through', (tester) async {
      final fake = _RecordFake();
      final app = AppStateStub(bridge: fake)..frontCompSelect('c0');
      app.beatSensitivity = 72;
      await tester.pumpWidget(_host(_trigger((context) {
        showLaneContextMenu(
          context: context,
          app: app,
          compId: 'c0',
          showTimeGrid: false,
          onToggleGrid: () {},
          onCompositionSettings: () async {},
          position: const Offset(20, 20),
        );
      })));
      await tester.tap(find.byKey(const Key('menu-trigger')));
      await tester.pump();
      await tester.tap(find.text('Detect beats'));
      await tester.pump();
      expect(fake.ops, contains('beats:c0:72'));
    });

    testWidgets('toggle grid fires the body callback', (tester) async {
      var toggled = false;
      final app = AppStateStub(bridge: _RecordFake());
      await tester.pumpWidget(_host(_trigger((context) {
        showLaneContextMenu(
          context: context,
          app: app,
          compId: 'c0',
          showTimeGrid: false,
          onToggleGrid: () => toggled = true,
          onCompositionSettings: () async {},
          position: const Offset(20, 20),
        );
      })));
      await tester.tap(find.byKey(const Key('menu-trigger')));
      await tester.pump();
      await tester.tap(find.text('Show time grid'));
      await tester.pump();
      expect(toggled, isTrue);
    });
  });

  group('keyframe interp menu widget', () {
    testWidgets('right-click menu commits Easy ease', (tester) async {
      final fake = _RecordFake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(_trigger((context) {
        showKeyframeInterpMenu(
          context: context,
          app: app,
          compId: 'c0',
          hit: _keyAt(60),
          hitId: const LaneKeyId('l0', 'position_x', 60),
          targets: {LaneKeyId('l0', 'position_x', 60)},
          position: const Offset(20, 20),
        );
      })));
      await tester.tap(find.byKey(const Key('menu-trigger')));
      await tester.pump();
      expect(find.text('Easy ease'), findsOneWidget);
      expect(find.text('Unify handles'), findsNothing); // linear key
      await tester.tap(find.text('Easy ease'));
      await tester.pump();
      expect(fake.ops.any((o) => o.contains('Bezier/Bezier')), isTrue);
    });
  });

  group('comp-strip pop out (row 3)', () {
    testWidgets('right-click the strip explains the Timeline stays docked',
        (tester) async {
      final app = AppStateStub(bridge: _RecordFake())..frontCompSelect('c0');
      await tester.pumpWidget(_host(TimelinePanel(app: app)));
      await tester.pump();
      // Right-click the comp pill; the strip's context menu opens. The popout
      // panel split keeps the Timeline in-window (it owns the transport +
      // preview cache), so the menu explains that rather than promising a popout.
      await tester.tap(find.text('Scene'), buttons: kSecondaryButton);
      await tester.pump();
      expect(find.text('Why can’t the Timeline pop out?'), findsOneWidget);
      await tester.tap(find.text('Why can’t the Timeline pop out?'));
      await tester.pump();
      expect(app.notice, contains('stays docked'));
    });
  });
}
