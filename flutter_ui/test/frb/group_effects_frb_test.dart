// Effects on a group header, on frb (K-731, docs/impl/group-effects.md §6-§7
// test 11): the header's stack crosses on the read model, the Timeline shows
// the fx tick exactly while it is non-empty and twirls real lanes under the
// header, a parameter edit round-trips through the shared instance lookup, and
// the Effect controls panel takes the group as its subject.
//
// Every document operation is genuine; see frb_test_support.dart.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Group header effects (frb)', () {
    ({
      LumitState state,
      LumitUiState uiState,
      CompositionReference comp,
      UuidValue groupId,
    }) withGroup({bool dressed = true}) {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      comp.addSolidLayer();
      final ids = [for (final l in comp.getLayers()) l.internallayerId];
      final gid = comp.groupLayers(layerIds: ids, name: 'Band');
      if (dressed) comp.addGroupEffect(group: gid, name: 'blur');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp, groupId: gid);
    }

    Future<void> mount(WidgetTester tester, dynamic p, Widget child) async {
      tester.view.physicalSize = const Size(1600, 700);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1600, 700),
        child: child,
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 6);
    }

    testWidgets('the header stack crosses resolved, and its fold rows root '
        'under the group prefix', (tester) async {
      final p = withGroup();
      final model = p.comp.getModel();
      final g = model.groups.single;
      expect(g.effects.single.name, 'blur');
      expect(
        model.layers.every((e) => e.info.effects.isEmpty),
        isTrue,
        reason: 'the wardrobe is the band\'s, not any member\'s',
      );

      final rows = groupHeaderFoldRows(group: g, open: {
        effectPath(groupFoldPrefix(g.id), g.effects.single.id.toString()),
      });
      expect(rows.whereType<FoldGroupRow>().single.label, 'Gaussian blur');
      final params = rows.whereType<FoldEffectParamRow>();
      expect(params, isNotEmpty, reason: 'an open heading shows its rows');
      expect(params.every((r) => r.group == g.id), isTrue);
      for (final r in params) {
        final path = foldRowPath('ignored-layer-id', r);
        expect(path, startsWith('${groupFoldPrefix(g.id)}/effects/'),
            reason: 'a group row roots under the group, whatever block it '
                'is drawn inside');
        expect(layerIdOfPath(path), isNot(anyOf(null, isEmpty)));
        expect(layerIdOfPath(path), startsWith('g:'),
            reason: 'a group prefix can never be mistaken for a layer id');
      }
    });

    testWidgets('the fx tick appears exactly while the stack is non-empty, '
        'and twirls the lanes open', (tester) async {
      final bare = withGroup(dressed: false);
      await mount(tester, bare, const TimelinePanelFrb());
      expect(
        find.byKey(ValueKey<String>('tl-group-fx-${bare.groupId}')),
        findsNothing,
        reason: 'an undressed header wears no tick',
      );

      final p = withGroup();
      await mount(tester, p, const TimelinePanelFrb());
      final tick = find.byKey(ValueKey<String>('tl-group-fx-${p.groupId}'));
      expect(tick, findsOneWidget);

      final fxId = p.comp.getModel().groups.single.effects.single.id;
      final headingKey = ValueKey<String>(
          'tl-keys-prop-${effectPath(groupFoldPrefix(p.groupId), '$fxId')}');
      expect(find.byKey(headingKey), findsNothing,
          reason: 'shut until the tick is pressed');
      await tester.tap(tick);
      await tester.pump();
      expect(find.byKey(headingKey), findsOneWidget,
          reason: 'the header twirls open like a layer, with real rows');
    });

    testWidgets('a group parameter edit round-trips through the shared '
        'instance lookup', (tester) async {
      final p = withGroup();
      final carrier = p.comp.getLayers().first;
      carrier.addEffect(name: 'invert');

      // Exactly what a row's write does: the list the row says it is on,
      // freshly read, the value staged on it, and the CARRIER layer's
      // setEffects as the commit — which the engine routes to the group.
      final staged = p.comp.getGroupEffects(group: p.groupId);
      staged.single.setValue(
        id: 'radius',
        value: const BridgeEffectValue.float(BridgeScalar.static_(42)),
      );
      carrier.setEffects(effects: staged);

      final model = p.comp.getModel();
      expect(
        model.groups.single.effects.single.values
            .firstWhere((v) => v.id == 'radius')
            .value,
        const BridgeEffectValue.float(BridgeScalar.static_(42)),
      );
      expect(
        model.layers.first.info.effects.single.name,
        'invert',
        reason: 'the carrier\'s own stack is untouched by the group write',
      );
    });

    testWidgets('the panel takes the header as its subject and adds to the '
        'group', (tester) async {
      final p = withGroup();
      p.uiState.selectedGroupHeader.value = p.groupId;
      await mount(tester, p, const EffectControlsPanelFrb());

      expect(find.text('Band'), findsOneWidget,
          reason: 'the header line names the group, the way it names a layer');
      expect(find.byKey(const ValueKey<String>('fx-card-group-0')),
          findsOneWidget, reason: 'the header\'s stack draws as cards');

      // Choosing a layer stands the group subject down again.
      p.uiState.setSelection([p.comp.getLayers().first]);
      expect(p.uiState.selectedGroupHeader.value, isNull);
    });
  });
}
