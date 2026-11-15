// The Timeline lays an effect's rows out as the Effect controls panel does:
// the schema's labelled groups as headings, and the Mix and Matte rows in a
// Compositing fold at the foot.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show cachedListParameterGroups;
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  /// [name] on a fresh layer: its rows with a set of paths twirled down, and
  /// its own path.
  (List<LayerFoldRow> Function(Set<String> open), String) effectOf(
      String name) {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    comp.addSolidLayer();
    final layer = comp.getLayers().single;
    layer.addEffect(name: name);
    final fx = layer.getEffects().single.getInfo();
    final values = {for (final v in fx.values) v.id: v.value};
    final path = effectPath('layer', '${fx.id}');
    return (
      (open) => effectFoldRows(fx, values,
          path: path,
          depth: 3,
          open: open,
          row: (param, depth) =>
              FoldEffectParamRow(fx, param, values[param.id], depth: depth)),
      path,
    );
  }

  List<String> ids(List<LayerFoldRow> rows) => [
        for (final r in rows)
          switch (r) {
            FoldEffectParamRow(:final param) => param.id,
            FoldGroupRow(:final label) => '[$label]',
            _ => '?',
          }
      ];

  testWidgets('Mix and Matte sit in a Compositing fold that starts shut',
      (tester) async {
    final (rows, path) = effectOf('blur');
    final shut = rows(const {});
    expect(ids(shut).last, '[Compositing]');
    expect(ids(shut), isNot(contains('mix')));
    expect(ids(shut), isNot(contains('matte')));
    expect(ids(shut), contains('radius'));

    final open = rows({effectGroupPath(path, 'compositing')});
    final listed = ids(open);
    final at = listed.indexOf('[Compositing]');
    expect(
        listed.sublist(at + 1),
        containsAllInOrder(
            ['mix', 'blend', 'matte', 'matte_invert', 'matte_channel']));
    final fold = open.sublist(at + 1).whereType<FoldEffectParamRow>();
    expect(fold.every((r) => r.depth == 4), isTrue,
        reason: 'the rows of a fold sit one step in');
  });

  testWidgets('a group the schema folds shut is a heading that starts shut',
      (tester) async {
    final groups = cachedListParameterGroups('glow');
    final g = groups.indexWhere((g) => g.label == 'Chromatic aberration');
    expect(groups[g].collapsed, isTrue);
    final hidden = groups[g].params.last;

    final (rows, path) = effectOf('glow');
    final shut = rows(const {});
    expect(ids(shut), contains('[Chromatic aberration]'));
    // After the loose rows, as the panel tucks it, and before Compositing.
    expect(
        ids(shut),
        containsAllInOrder(
            ['rolloff', '[Chromatic aberration]', '[Compositing]']));
    expect(ids(shut), isNot(contains(hidden)));

    final open = rows({effectGroupPath(path, '$g')});
    expect(ids(open), contains(hidden));

    // A reveal builds the rows with every twirl down.
    final every = rows(everyFoldPath);
    expect(ids(every), containsAll([hidden, 'mix', 'matte']));
  });

  testWidgets('a group the schema leaves open starts open and can be shut',
      (tester) async {
    // Any built-in with a labelled group that starts open.
    final name = listEffects()
        .map((e) => e.name)
        .firstWhere((n) => cachedListParameterGroups(n)
            .any((g) =>
                g.label.isNotEmpty &&
                !g.collapsed &&
                g.visibleWhenParam == null));
    final groups = cachedListParameterGroups(name);
    final g = groups.indexWhere((g) =>
        g.label.isNotEmpty && !g.collapsed && g.visibleWhenParam == null);
    printOnFailure('$name: ${groups[g].label}');

    final (rows, path) = effectOf(name);
    final open = rows(const {});
    final heading = open.whereType<FoldGroupRow>().firstWhere(
        (r) => r.path == effectGroupPath(path, '$g:shut'));
    expect(heading.open, isTrue);

    final shut = rows({heading.path});
    expect(
        shut
            .whereType<FoldGroupRow>()
            .firstWhere((r) => r.path == heading.path)
            .open,
        isFalse);
    expect(shut.length, lessThan(open.length));
  });

  testWidgets('a gradient has the Compositing fold too', (tester) async {
    final rows = effectOf('gradient').$1(const {});
    expect(ids(rows).last, '[Compositing]');
    expect(ids(rows), isNot(contains('mix')));
  });
}
