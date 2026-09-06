// Adopting another project is announced on the scoped-change stream.
//
// Every per-document cache in the shell drops itself on an `items` change and
// on nothing else: the Project panel's item, name and probe caches, the comp
// read model, the comp-time cache. Swapping the project without saying so left
// all of them holding the previous document — whose handles the swap has just
// closed — and the first build that touched one threw, which in a release build
// is a blank grey panel and no message anywhere.
//
// So the regression to hold is not "the caches are right", which each panel
// tests for itself; it is that the swap is *published at all*. Without the fix
// no event arrives and these two time out.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets('a second new project is published as an items change',
      (tester) async {
    final state = LumitState()..newProject();
    final seen = <ScopedChange>[];
    final sub = state.onChange.listen(seen.add);
    addTearDown(sub.cancel);

    state.newProject();
    await tester.pump();

    expect(seen, isNotEmpty,
        reason: 'the swap must reach the panels that cache per document');
    expect(seen.single.items, isTrue,
        reason: 'a different tree is an items change, which is the one scope '
            'the Project panel and the comp read model listen for');
  });

  testWidgets('the published change names no item or layer', (tester) async {
    // The scope matters as much as the event. An `item` or a `layer` on it
    // would be a reference into the project being *replaced*, and the two
    // subscribers that compare against one — ProjectItemBuilder and the comp
    // read model — would call straight into the document just closed. Broad is
    // correct here: nothing below the root survived the swap.
    final state = LumitState()..newProject();
    final seen = <ScopedChange>[];
    final sub = state.onChange.listen(seen.add);
    addTearDown(sub.cancel);

    state.newProject();
    await tester.pump();

    expect(seen.single.item, isNull);
    expect(seen.single.layer, isNull);
  });
}
