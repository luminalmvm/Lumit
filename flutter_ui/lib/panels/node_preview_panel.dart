// The Node preview panel: the picture **at** whichever box the Graph panel has
// picked (K-448, K-486, docs/impl/node-graph.md §8 WP5). Its face follows the
// approved Nodes-workspace drawing's panel family — the same 22px strip, the
// same subject-in-the-header shape as the Node panel beside it — since that
// drawing carries the Node preview as a mode of its small viewer rather than as
// a panel of its own.
//
// **In plain terms.** The Graph panel draws a layer's effects as a row of
// boxes. This panel answers "what does the picture look like *here*" — at one
// of those boxes rather than at the end of the chain. Pick the blur and you see
// the picture with the blur applied and nothing after it, without switching
// anything off and without soloing the layer: the Viewer keeps showing the
// composition while this shows the point in the chain you are working on.
//
// **Where the picture comes from.** One call, `previewNode`, answered on the
// stream the frames and traces already ride. The engine renders the composition
// with that layer's stack cut off at the picked box and sends back a small
// still — a thumbnail, bounded to 256px on its longest edge, not a second
// Viewer (K-183: frames stream as GPU handles, and this does not stream). It is
// asked for when the pick, the playhead, the layer or the document moves, and
// never in a rebuild — `bridge_call_budget_test` expects a hover here to cost
// nothing at all.
//
// **What has no picture.** A driver makes a number, not a picture, so picking
// one draws the empty face rather than asking for something that is not coming.
// So does picking nothing.

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'graph_panel.dart' show graphToolbarHeight;
import 'placeholder.dart';

/// The longest edge asked for, matching the engine's own cap
/// (`worker_thread::MAX_PREVIEW_EDGE`). A preview is a postage stamp beside the
/// Viewer, and the cap is what keeps this a bounded thumbnail rather than a
/// second frame transport.
const int nodePreviewMaxEdge = 256;

/// How a node is spelt on the seam: `source`, `out`, or the effect instance's
/// id. `null` for anything that makes no picture — a driver, or no pick at all.
String? nodePreviewSpelling(BridgeNodeRef? node) => switch (node) {
      BridgeNodeRef_Source() => 'source',
      BridgeNodeRef_Out() => 'out',
      BridgeNodeRef_Effect(:final field0) => field0.toString(),
      _ => null,
    };

class NodePreviewPanelFrb extends StatefulWidget {
  const NodePreviewPanelFrb({super.key});

  @override
  State<NodePreviewPanelFrb> createState() => _NodePreviewPanelFrbState();
}

class _NodePreviewPanelFrbState extends State<NodePreviewPanelFrb> {
  LumitUiState? _ui;

  /// The reply channel. Cancelled with the panel, which is the whole of
  /// "closing the panel stops the second render": nothing is left asking.
  StreamSubscription<WorkerResponse>? _replies;

  /// The picked box's name, read once per change beside the ask — never in a
  /// build.
  String? _nodeName;

  /// What was last asked for, as the engine spells it. A reply naming anything
  /// else arrived after the pick moved on and is dropped rather than drawn.
  String? _wanted;

  /// The picture, and the node it is of. Kept while a newer one is being made,
  /// so the panel holds the last good picture rather than blanking on every
  /// playhead step.
  ui.Image? _picture;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final state = Provider.of<LumitUiState>(context, listen: false);
    if (identical(state, _ui)) return;
    _unbind();
    _ui = state;
    state.selectedLayer.addListener(_reload);
    state.graphNode.addListener(_reload);
    state.model.addListener(_reload);
    state.playheadFrame.addListener(_ask);
    // A rendered frame reaching the Viewer is the other reason to ask again: a
    // value drag holds the playhead still and changes the picture under it, so
    // without this the preview kept showing the picture from before the drag.
    state.frameArrived.addListener(_ask);
    _replies = Provider.of<LumitState>(context, listen: false)
        .onWorkerResponse
        .listen(_reply);
    _reload();
  }

  void _unbind() {
    _ui?.selectedLayer.removeListener(_reload);
    _ui?.graphNode.removeListener(_reload);
    _ui?.model.removeListener(_reload);
    _ui?.playheadFrame.removeListener(_ask);
    _ui?.frameArrived.removeListener(_ask);
    _replies?.cancel();
    _replies = null;
  }

  @override
  void dispose() {
    _unbind();
    _picture?.dispose();
    super.dispose();
  }

  /// The pick or the layer moved: re-read the box's name and ask afresh. The
  /// held picture goes with the pick — showing the last node's picture under a
  /// new node's name would be a lie the eye cannot catch.
  void _reload() {
    if (!mounted) return;
    final layer = _ui?.selectedLayer.value;
    final node = _ui?.graphNode.value;
    String? name;
    if (layer != null && node != null) {
      try {
        for (final box in layer.getGraph().nodes) {
          if (box.node == node) {
            name = box.customName ?? engineLabel(box.label);
            break;
          }
        }
      } catch (_) {
        name = null; // the layer went away since it was picked
      }
    }
    setState(() {
      _nodeName = name;
      _picture?.dispose();
      _picture = null;
      _wanted = null;
    });
    _ask();
  }

  /// Ask for the picture at the picked box. Silent for a box that makes none.
  void _ask() {
    final state = _ui;
    final layer = state?.selectedLayer.value;
    final comp = state?.selectedComp;
    final node = nodePreviewSpelling(state?.graphNode.value);
    if (state == null || layer == null || comp == null || node == null) return;
    _wanted = node;
    try {
      comp.previewNode(
        frame: BigInt.from(state.playheadFrame.value),
        layer: layer,
        node: node,
        maxEdge: nodePreviewMaxEdge,
      );
    } catch (_) {
      // No worker to ask yet, or the comp has gone. The panel keeps what it
      // has and asks again at the next change; a preview is never worth taking
      // the shell down for.
    }
  }

  void _reply(WorkerResponse msg) {
    if (msg is! WorkerResponse_NodePreview) return;
    final preview = msg.field0;
    if (!mounted || preview.node != _wanted) return;
    if (preview.width == 0 || preview.height == 0) return;
    ui.decodeImageFromPixels(
      preview.rgba,
      preview.width,
      preview.height,
      ui.PixelFormat.rgba8888,
      (image) {
        if (!mounted || preview.node != _wanted) {
          image.dispose();
          return;
        }
        setState(() {
          _picture?.dispose();
          _picture = image;
        });
      },
    );
  }

  // --- Drawing -------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final name = _nodeName;
    final node = _ui?.graphNode.value;
    if (name == null) {
      return PlaceholderPanel(
        icon: LumitIcon.nodes,
        title: l10n.panelNodePreview,
        hint: l10n.nodePreviewNoSelection,
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _header(context, name),
        Expanded(
          child: nodePreviewSpelling(node) == null
              ? PlaceholderPanel(
                  icon: LumitIcon.nodes,
                  title: l10n.panelNodePreview,
                  hint: l10n.nodePreviewNoPicture,
                )
              : _stage(context),
        ),
      ],
    );
  }

  /// The panel's own strip, the Node panel's shape: the picked box's name, on
  /// the same 22px `surface1` band, so the two read as one family whichever
  /// column they land in.
  Widget _header(BuildContext context, String name) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('node-preview-header'),
      height: graphToolbarHeight,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      child: Row(
        children: [
          Expanded(
            child: Text(
              name,
              key: const ValueKey('node-preview-name'),
              style: t.body,
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }

  /// The picture on its mat: centred, whole, and never cropped — a preview that
  /// filled the panel would hide the very edge an effect is usually judged by.
  /// The mat is the Viewer's own pasteboard token, so the two pictures sit on
  /// the same ground.
  Widget _stage(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final picture = _picture;
    return Container(
      key: const ValueKey('node-preview-stage'),
      color: t.viewerSurround,
      padding: const EdgeInsets.all(8),
      child: picture == null
          ? const SizedBox.expand()
          : RawImage(
              key: const ValueKey('node-preview-picture'),
              image: picture,
              fit: BoxFit.contain,
            ),
    );
  }
}
