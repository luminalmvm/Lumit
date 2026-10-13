// What a drag carries between panels.
//
// Each of these types is the *contract* between one panel that produces a drag
// and another that accepts it: nothing else produces a `FootageDragData`, and
// the Timeline's drop target accepts nothing else. Changing a payload therefore
// breaks a gesture silently — the drop simply stops matching — which is why they
// live here together rather than beside whichever panel happened to need one
// first.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show readEffectPreset;
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart' show UuidValue;

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

/// An effect, or a saved preset, dragged from the Effects & presets panel
/// onto a layer or a clip.
class EffectDragData {
  /// The stable match name `addEffect` takes. Empty for a preset.
  final String name;
  final String label;

  /// The preset's file, for a saved preset.
  final String? presetPath;
  const EffectDragData(this.name, this.label) : presetPath = null;
  const EffectDragData.preset(String path, this.label)
      : name = '',
        presetPath = path;

  /// Put what was dragged onto [layer]'s stack, or with [clip] onto that
  /// clip's own. Quiet when the engine refuses or the file has gone: the
  /// drop then simply did nothing.
  Future<void> landOn(LayerReference layer, {UuidValue? clip}) async {
    try {
      if (presetPath case final path?) {
        layer.loadPreset(text: await readEffectPreset(path: path), clip: clip);
      } else if (clip != null) {
        layer.addClipEffect(clip: clip, name: name);
      } else {
        layer.addEffect(name: name);
      }
    } catch (_) {}
  }
}
