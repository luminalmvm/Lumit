// What a drag carries between panels.
//
// Each of these types is the *contract* between one panel that produces a drag
// and another that accepts it: nothing else produces a `FootageDragData`, and
// the Timeline's drop target accepts nothing else. Changing a payload therefore
// breaks a gesture silently — the drop simply stops matching — which is why they
// live here together rather than beside whichever panel happened to need one
// first.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';

/// Footage dragged from the Project panel onto the Timeline, or onto the New
/// composition button.
///
/// It carries the handles themselves, not ids to look them back up by: on frb the
/// reference *is* the identity, so the drop calls `addFootageLayer` with what it
/// was given and never has to search the project for it.
///
/// A *list*, because the Project panel selects more than one row: dragging any
/// row of a multi-selection brings the whole selection, which is what makes
/// "drop four clips on New composition" a single gesture. A single-item drag is
/// the same payload with one entry, so the drop targets have one path.
class FootageDragData {
  final List<FootageReference> footage;

  /// What the floating label under the pointer reads: the item's name for one,
  /// a count for several.
  final String label;
  const FootageDragData(this.footage, this.label);
}

/// A composition dragged from the Project panel onto another comp's Timeline,
/// where the drop nests it as a Precomp layer. One comp, not a list: nesting
/// is a deliberate act on a specific comp, not a batch operation.
class CompDragData {
  final CompositionReference comp;

  /// What the floating label under the pointer reads.
  final String label;
  const CompDragData(this.comp, this.label);
}

/// An effect dragged from the Effects & presets panel onto a layer.
class EffectDragData {
  /// The stable match name `addEffect` takes.
  final String name;
  final String label;
  const EffectDragData(this.name, this.label);
}
