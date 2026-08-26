// The Timeline's Retime row (K-197).
//
// Split out of timeline_panel_frb.dart, where its doc comment had drifted from
// its class; the two are back together here.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';
import 'graph_editor_frb.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';
import 'keyframe_controls_frb.dart';
import 'timeline_metrics_frb.dart';

/// The layer's Retime (K-197): which moment of the source, in seconds, the
/// layer shows at this point on its own timeline.
///
/// An ordinary property row — the same stopwatch, the same navigator, the same
/// lane diamonds and the same graph lanes as Position. It sits above Transform
/// and only exists while the layer has been given a Retime (Ctrl+Alt+T), so
/// unlike Volume its scalar arrives on the fold row rather than being read here
/// (K-184: no bridge calls while drawing).
class RetimeRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final BridgeScalar scalar;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// Selects the channel, so its curve opens in the graph — the same handle
  /// every other property row's name is. Retime was built without one, which
  /// left it the one channel `graphChannels` could build and nobody could
  /// choose.
  final VoidCallback? onLabelTap;

  const RetimeRow({super.key, 
    required this.comp,
    required this.layer,
    required this.scalar,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.onLabelTap,
  });

  @override
  State<RetimeRow> createState() => _RetimeRowState();
}

class _RetimeRowState extends State<RetimeRow> {
  /// The value under the pointer during a drag, held so the whole gesture is
  /// one undo step. The picture keeps up in the meantime: a retime drag decides
  /// which frame is decoded, so it previews through its own door
  /// (`renderFrameWithRetime`) rather than by re-compositing pixels already in
  /// hand — the one edit where watching it move is the whole point.
  double? _staged;

  final PreviewThrottle _preview = PreviewThrottle();

  @override
  void dispose() {
    _preview.cancel();
    super.dispose();
  }

  /// The footage's own rate, probed once when the row mounts. Null until the
  /// probe answers, or when the source is not footage (or carries no video
  /// stream) — the comp rate stands in then, so the clock is always usable.
  (int, int)? _sourceFps;

  @override
  void initState() {
    super.initState();
    _probeSourceFps();
  }

  Future<void> _probeSourceFps() async {
    final item = widget.layer.getSourceItem();
    if (item is! ItemReference_Footage) return;
    final info = await item.field0.mediaInfo();
    if (!mounted || info == null || info.fpsNum <= 0 || info.fpsDen <= 0) {
      return;
    }
    setState(() => _sourceFps = (info.fpsNum, info.fpsDen));
  }

  /// Whether this gesture already planted its key — one plant per drag.
  bool _planted = false;

  /// A drag tick: render the map the release will write, without writing it —
  /// and publish it, so the graph's Retime curve follows the drag (K-334).
  ///
  /// The first tick on a frame with **no key plants one** holding the value
  /// already showing (K-333's rule, K-336 for this row): nothing moves, and
  /// the preview then *replaces* a real key instead of inserting beside the
  /// document's — the aligned path the transform rows take.
  void _live(BridgeScalar scalar, double value, int frame) {
    if (!_planted &&
        scalar is BridgeScalar_Keyframed &&
        !scalar.field0
            .any((k) => widget.comp.frameAtTime(time: k.time) == frame)) {
      _planted = true;
      final held = sampleScalar(
          scalar: scalar, time: widget.comp.timeOfFrame(frame: frame));
      widget.layer.setRetimeProperty(
        value: scalarWithValueAt(scalar, held, widget.comp, frame),
      );
      widget.onChanged();
    }
    setState(() => _staged = value);
    rowValueDrag.value = RowValueDrag(
      layer: widget.layer.internallayerId.toString(),
      retime: true,
      frame: frame,
      value: value,
    );
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _preview.request(() => widget.comp.renderFrameWithRetime(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          retime: scalarWithValueAt(scalar, value, widget.comp, frame),
        ));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final scalar = widget.scalar;
    final animated = scalar is BridgeScalar_Keyframed;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final playhead = ui.playheadFrame;
    // Which face the row wears (K-287): the clock by default, seconds for
    // anyone who asked for them in Settings ▸ Interface ▸ Editing.
    final seconds = ui.workspace.interface.retimeInSeconds;
    // The clock face counts *source* frames, so it runs at the footage's own
    // rate — 600 fps footage counts to :599 whatever the comp's rate says.
    final (fpsNum, fpsDen) = _sourceFps ?? ui.model.fpsExact;

    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) {
        final value = _staged ??
            (animated
                ? sampledScalar(scalar, timeOfFrame(widget.comp, frame))
                : (scalar as BridgeScalar_Static).field0);
        return Row(
          children: [
            KeyframeControlsFrb(
              scalars: [scalar],
              comp: widget.comp,
              playheadFrame: frame,
              onSeek: widget.onSeek,
              rowKey: 'tl-retime',
              onWrite: (next) {
                widget.layer.setRetimeProperty(value: next.single);
                widget.onChanged();
              },
            ),
            const SizedBox(width: 4),
            Expanded(
              child: GestureDetector(
                key: const ValueKey('tl-retime-name'),
                behavior: HitTestBehavior.opaque,
                onTap: widget.onLabelTap,
                child: Text(l10n.retime, style: t.body),
              ),
            ),
            SizedBox(
              width: widget.valueColumn.width,
              child: seconds
                  ? (animated
                      ? KeyedValueField(
                          fieldKey: const ValueKey('tl-retime-seconds'),
                          onLive: (v) => _live(scalar, v, frame),
                          value: value,
                          // The same open range a transform axis gets: a
                          // source time before zero or past the end simply
                          // holds the end frame (docs/04 §7), so clamping the
                          // field would only fight the drag.
                          min: -100000,
                          max: 100000,
                          decimals: 3,
                          suffix: ' s',
                          speed: 0.02,
                          onCommit: (v) => _commitAt(scalar, v, frame),
                        )
                      : DragValueField(
                          key: const ValueKey('tl-retime-seconds'),
                          value: value,
                          min: -100000,
                          max: 100000,
                          decimals: 3,
                          suffix: ' s',
                          speed: 0.02,
                          onChanged: (v) => _commitAt(scalar, v, frame),
                          onChangeLive: (v) =>
                              _live(scalar, v.toDouble(), frame),
                          onChangeEnd: (v) => _commitAt(scalar, v, frame),
                          onDragCancel: () => setState(() => _staged = null),
                        ))
                  // The clock face (K-287, realising K-075): which moment of
                  // the source is showing, written the way every other time in
                  // the editor is written. Dragged and typed in whole source
                  // frames — a timecode cannot say "between two frames", which
                  // is what the seconds setting is for.
                  : TimeReadout(
                      key: const ValueKey('tl-retime-seconds'),
                      frame: _frameOfSeconds(value, fpsNum, fpsDen),
                      format: (f) => timecodeOfRateSigned(f, fpsNum, fpsDen),
                      parse: (text) =>
                          framesOfTimecodeSigned(text, fpsNum, fpsDen),
                      widthChars: timecodeChars(fpsNum, fpsDen) + 1,
                      // A value in a property row, so the value well's own
                      // 11px mono (§12A.6) — `t.mono` bare is 12, a size no
                      // mockup draws anywhere and the one number in the
                      // Timeline that still read a step larger than the rest.
                      style: t.mono.copyWith(fontSize: wellTextSize),
                      // **In a well, like every other property row's value**
                      // (K-460's rule, applied here at last): the clock face
                      // is dragged and typed into exactly as the seconds face
                      // beside it is, and that face has always been a
                      // `DragValueField` with a recess round it. This one was
                      // bare text that happened to answer a drag, which is
                      // the one thing K-460 says a value must never be.
                      well: true,
                      minFrame: -100000,
                      maxFrame: 100000,
                      draggable: true,
                      onDragLive: (f) => _live(
                          scalar, _secondsOfFrame(f, fpsNum, fpsDen), frame),
                      onCommit: (f) => _commitAt(
                          scalar, _secondsOfFrame(f, fpsNum, fpsDen), frame),
                      onDragCancel: () => setState(() => _staged = null),
                    ),
            ),
            SizedBox(width: widget.valueColumn.rightInset),
          ],
        );
      },
    );
  }

  /// A source time in seconds as a whole source frame, and back.
  ///
  /// At the footage's own rate where the source is footage whose rate is
  /// known; at the composition's rate until the probe answers, and for
  /// everything else.
  static int _frameOfSeconds(double seconds, int fpsNum, int fpsDen) {
    if (fpsDen <= 0 || fpsNum <= 0) return 0;
    return (seconds * fpsNum / fpsDen).round();
  }

  static double _secondsOfFrame(int frame, int fpsNum, int fpsDen) =>
      fpsNum <= 0 ? 0 : frame * (fpsDen <= 0 ? 1 : fpsDen) / fpsNum;

  void _commitAt(BridgeScalar scalar, num value, int frame) {
    // The write is the last word on the gesture: a held preview tick after it
    // would put the provisional picture back.
    _preview.cancel();
    rowValueDrag.value = null;
    _planted = false;
    widget.layer.setRetimeProperty(
      value: scalarWithValueAt(scalar, value.toDouble(), widget.comp, frame),
    );
    setState(() => _staged = null);
    widget.onChanged();
  }
}
