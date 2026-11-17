// Copies of a graphics template: what a copy is called where its name is
// shown, and the request that puts the cursor in a copy's first text box.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:lumit_flutter/src/rust/api/template/copy.dart';
import 'package:uuid/uuid.dart';

import 'dock.dart';

/// What a layer is called where its name is shown. A copy whose name is still
/// the template's goes by what its first text control says, so twenty copies
/// read as twenty names. The layer's stored name is not changed.
String shownLayerName(BridgeLayerInfo info) {
  final copy = info.copy;
  if (copy == null || info.name != copy.name) return info.name;
  return copy.title ?? info.name;
}

/// What a clip is called on its bar in the Cut timeline.
String shownClipName(BridgeClip clip) =>
    clip.angle?.name ?? clip.copy?.title ?? clip.sourceName;

/// The ids of a copy's rows that show nothing in a Timeline lane: words, a
/// font, a picture and how it is fitted.
Set<String> copyRowsWithoutLane(BridgeCopy copy) => {
      for (final c in copy.controls) ...[
        if (c.kind == BridgeControlKind.text ||
            c.kind == BridgeControlKind.font ||
            c.kind == BridgeControlKind.footage)
          c.id,
        '${c.id}_font',
        '${c.id}_fit',
      ],
    };

/// Whether a copy has words to type: a text control.
bool copyHasWords(BridgeCopy? copy) =>
    copy != null &&
    copy.controls.any((c) => c.kind == BridgeControlKind.text);

/// The copy whose first text box is asked to take the cursor: its layer, and
/// its clip when it is one. The copy's card in Effect controls answers and
/// puts this back to null.
final ValueNotifier<({UuidValue layer, UuidValue? clip})?> copyWordsFocus =
    ValueNotifier(null);

/// Put the cursor in the first text box of a copy's card in Effect controls,
/// with its words selected. The panel is shown where the layout has it shut.
/// The copy is the selected layer, or [clip] of it.
void focusCopyWords(LumitUiState ui, LayerReference layer, {UuidValue? clip}) {
  if (!panelVisible(ui.split, Panel.effectControls)) {
    setPanelVisible(ui.split, Panel.effectControls, true);
    ui.workspace.touch();
  }
  ui.frontPanel(Panel.effectControls);
  final asked = (layer: layer.internallayerId, clip: clip);
  copyWordsFocus.value = asked;
  // Dropped when no card has taken it two frames on, so it cannot move the
  // cursor at some later time.
  final frames = WidgetsBinding.instance;
  frames.addPostFrameCallback((_) {
    frames.addPostFrameCallback((_) {
      if (copyWordsFocus.value == asked) copyWordsFocus.value = null;
    });
    frames.ensureVisualUpdate();
  });
  frames.ensureVisualUpdate();
}
