// What the Text and Paragraph panels edit, and how an edit reaches it.
//
// With text layers selected the panels edit those layers. With none selected
// they edit what the Type tool sets new text in, which is how After Effects'
// Character panel works too.
//
// A value being dragged is shown through the text preview path, the same one
// typing uses, and the document is written once when the drag ends. One drag
// is then one undo step, and several selected layers are one step too.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../state/preview_throttle.dart';
import '../state/text_documents.dart';
import '../state/tools.dart';
import '../theme/theme.dart';

/// One change to a text document: the document in, the changed one out.
typedef TextChange = BridgeTextDocument Function(BridgeTextDocument document);

/// What a panel is editing this build.
class TextTarget {
  /// The selected text layers, top first. Empty when the panel is editing
  /// what new text is set in.
  final List<LayerReference> layers;

  /// The first layer's name, or null with no text layer selected.
  final String? name;

  /// The document the controls show: the first layer's, or one made of the
  /// Type tool's options. While a value is being dragged it is the document
  /// the drag has reached, so the number under the pointer moves.
  final BridgeTextDocument document;

  /// The document as it is held, which every change starts from.
  final BridgeTextDocument held;

  const TextTarget({
    required this.layers,
    required this.name,
    required this.document,
    required this.held,
  });
}

/// The reading and writing both panels share.
mixin TextTargetState<T extends StatefulWidget> on State<T> {
  final PreviewThrottle _throttle = PreviewThrottle();

  /// Where a drag has got to, between its first tick and its end.
  BridgeTextDocument? _staged;

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  /// What the panel is editing. Read each build, so it follows the selection
  /// and anything else that changes the layer.
  TextTarget targetOf(LumitUiState ui) {
    final selected = ui.selectedLayerIds;
    final layers = <LayerReference>[];
    String? name;
    BridgeTextDocument? document;
    for (final entry in ui.model.heldLayers) {
      if (entry.info.kind != BridgeLayerKind.text) continue;
      if (!selected.contains(entry.layer.internallayerId)) continue;
      try {
        // The first one's document is the one shown, and the only one read:
        // this runs on every build, and a drag with thirty layers selected
        // was thirty reads across the bridge for each move of the pointer.
        if (document == null) {
          final read = entry.layer.getText();
          if (read == null) continue;
          document = read;
        }
        name ??= entry.info.name;
        layers.add(entry.layer);
      } catch (_) {
        // The layer went away between the model being read and this call.
      }
    }
    final held = document ?? _newText(ui.tools);
    return TextTarget(
      layers: layers,
      name: name,
      document: _staged ?? held,
      held: held,
    );
  }

  BridgeTextDocument _newText(ToolsState tools) => BridgeTextDocument(
        text: '',
        size: tools.textSize,
        fill: tools.fillRgba,
        pathOffset: const BridgeScalar.static_(0),
        animators: const [],
        style: tools.textStyle,
        paragraph: tools.paragraphStyle,
      );

  /// Show [change] on the picture without writing it. Only the first selected
  /// layer is previewed, the rest follow when the drag ends.
  void previewText(LumitUiState ui, TextTarget target, TextChange change) {
    final document = change(target.held);
    setState(() => _staged = document);
    final comp = ui.selectedComp;
    if (target.layers.isEmpty || comp == null) return;
    final layer = target.layers.first;
    _throttle.request(() {
      try {
        ui.liveText.value = {layer.internallayerId: document};
        comp.renderFrameWithTextPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: layer,
          document: document,
        );
      } catch (_) {
        // A preview is a courtesy, the edit carries on without it.
      }
    });
  }

  /// Put the picture back to what the layer holds, after a drag that came to
  /// nothing.
  void cancelTextPreview(LumitUiState ui, TextTarget target) {
    _throttle.cancel();
    setState(() => _staged = null);
    if (ui.liveText.value.isNotEmpty) ui.liveText.value = const {};
    final comp = ui.selectedComp;
    if (target.layers.isEmpty || comp == null) return;
    try {
      comp.renderFrameWithTextPreview(
        frame: BigInt.from(ui.playheadFrame.value),
        scale: ui.viewerScale,
        layer: target.layers.first,
        document: target.held,
      );
    } catch (_) {
      // The layer went away mid-drag, there is nothing to put back.
    }
  }

  /// Write [change] to every selected text layer as one undo step, or to the
  /// Type tool's options when none is selected.
  void commitText(LumitUiState ui, TextTarget target, TextChange change) {
    _throttle.cancel();
    if (_staged != null) setState(() => _staged = null);
    if (ui.liveText.value.isNotEmpty) ui.liveText.value = const {};
    if (target.layers.isEmpty) {
      final next = change(target.held);
      ui.tools
        ..textSize = next.size
        ..fill = ToolColour(next.fill.r, next.fill.g, next.fill.b)
        ..textStyle = next.style
        ..paragraphStyle = next.paragraph;
      return;
    }
    final project = Provider.of<LumitState>(context, listen: false).project;
    final group = target.layers.length > 1 && project != null;
    if (group) project.beginUndoGroup();
    try {
      for (final layer in target.layers) {
        try {
          final current = layer.getText();
          if (current != null) layer.setText(document: change(current));
        } catch (_) {
          // The layer was deleted while the panel still showed it.
        }
      }
    } finally {
      if (group) project.endUndoGroup();
    }
    ui.model.refresh();
  }
}

/// Changes to the style alone, which is most of what the Text panel writes.
TextChange restyle(BridgeTextStyle Function(BridgeTextStyle style) change) =>
    (document) => document.copyWith(style: change(document.style));

/// Changes to the paragraph alone.
TextChange reflow(
        BridgeParagraphStyle Function(BridgeParagraphStyle paragraph) change) =>
    (document) => document.copyWith(paragraph: change(document.paragraph));

/// The heading both panels open with: the layer being edited, or that it is
/// new text.
Widget textTargetHeading(LumitTheme t, String label) => Padding(
      padding: const EdgeInsets.fromLTRB(10, 8, 10, 4),
      child: Text(
        label,
        style: t.mono.copyWith(fontSize: 10, color: t.textPrimary),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
    );

/// A label and its control, as one cell of a panel's grid.
Widget textCell(LumitTheme t, String label, Widget control) => SizedBox(
      height: t.density.propertyRow,
      child: Row(
        children: [
          const SizedBox(width: 10),
          SizedBox(
            width: 92,
            child: Text(
              label,
              style: t.body.copyWith(color: t.textMuted),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          Expanded(child: Align(alignment: Alignment.centerLeft, child: control)),
          const SizedBox(width: 10),
        ],
      ),
    );

/// The panels' cells laid out in two columns where there is room for two, and
/// one where there isn't.
Widget textGrid(List<Widget> cells) => LayoutBuilder(
      builder: (context, constraints) {
        final columns = constraints.maxWidth >= 380 ? 2 : 1;
        final width = constraints.maxWidth / columns;
        return Wrap(
          children: [for (final cell in cells) SizedBox(width: width, child: cell)],
        );
      },
    );
