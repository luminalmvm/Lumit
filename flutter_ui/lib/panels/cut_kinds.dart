// The kinds an overlap can be, asked of the engine once, since a panel's build
// reads them and the list never changes.

import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart' show asOneUndoStep;
import 'package:lumit_flutter/shell/menu_bar_frb.dart' show MenuEntry;
import 'package:lumit_flutter/src/rust/api/clip_edits.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

final List<BridgeTransitionKind> transitionKinds = cutTransitionKinds();

/// What a kind is called: its one look's name, or the effect's.
String kindLabel(BridgeTransitionKind kind) =>
    engineLabel(kind.looks.length == 1 ? kind.looks.first : kind.label);

/// Layer ▸ Transition: a flyout per group of kinds, a kind with several looks
/// a flyout of them, each putting a transition between the two layers
/// [selected] holds as one undo step. Greyed unless it holds exactly two,
/// neither locked.
MenuEntry transitionMenu(LumitState app, CompositionReference? comp,
    List<BridgeLayerEntry> selected) {
  final open = [
    for (final e in selected)
      if (!e.info.switches.locked) e.layer.internallayerId,
  ];
  if (comp == null || selected.length != 2 || open.length != 2) {
    return MenuEntry(l10n.menuLayerTransition, null);
  }
  void put(BridgeTransitionKind kind, int? look) {
    var done = false;
    asOneUndoStep(app.project, () {
      try {
        done = comp.transitionBetween(
            first: open[0], second: open[1], kind: kind.matchName, look: look);
      } catch (_) {}
    }, name: 'Transition');
    if (done) {
      app.notifyDocumentChanged();
    } else {
      app.postNotice(l10n.menuLayerTransitionNoOverlap);
    }
  }

  MenuEntry row(BridgeTransitionKind kind) {
    final looks = kind.looks;
    if (looks.length < 2) {
      return MenuEntry(kindLabel(kind), () => put(kind, null));
    }
    return MenuEntry.submenu(engineLabel(kind.label), [
      for (var i = 0; i < looks.length; i++)
        MenuEntry(engineLabel(looks[i]), () => put(kind, i)),
    ]);
  }

  final groups = [
    l10n.cutTransitionWipes,
    l10n.cutTransitionMoves,
    l10n.cutTransitionHits,
  ];
  return MenuEntry.submenu(l10n.menuLayerTransition, [
    for (var g = 0; g < groups.length; g++)
      if (transitionKinds.any((kind) => kind.group == g))
        MenuEntry.submenu(groups[g], [
          for (final kind in transitionKinds)
            if (kind.group == g) row(kind),
        ]),
  ]);
}
