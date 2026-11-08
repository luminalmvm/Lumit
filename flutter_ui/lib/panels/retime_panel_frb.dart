// The Retime panel: how fast the selected layer plays, how long that makes it,
// and the commands that change it. A view over the layer's one Retime, so the
// graph, the Timeline's row and this panel can never disagree.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';
import 'graph_maths.dart';
import 'retime_commands.dart';
import 'timeline_extras_frb.dart' show Segment, SegmentOption;

/// The speeds a layer is most often put at, one click each.
const List<int> _speedChips = [25, 50, 100, 200, 400];

/// Below this the speed and the duration stop sharing a row.
const double _narrow = 300;
const double _labelWidth = 56;

/// The strip takes what the panel's height leaves, between these. Everything
/// else in the panel stands at about [_besideStrip] tall.
const double _stripLeast = 56;
const double _stripMost = 300;
const double _besideStrip = 176;

class RetimePanelFrb extends StatefulWidget {
  const RetimePanelFrb({super.key});

  @override
  State<RetimePanelFrb> createState() => _RetimePanelFrbState();
}

class _RetimePanelFrbState extends State<RetimePanelFrb> {
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);
  late final Listenable _changes =
      Listenable.merge([_ui.model, _ui.selectedLayers]);

  /// The speed a drag on the well is asking for, shown while it runs and
  /// written once when it ends.
  double? _liveSpeed;

  /// How the first target finds its in-between frames. The read model only
  /// says whether flow is on, so this is asked once per document revision, and
  /// never by a rebuild that changed nothing.
  BridgeRetimeInterp? _interp;
  Object? _interpAsked;

  /// The footage's own rate by layer id, which the source clock counts in, as
  /// the Retime row's does. Asked once per layer, and never from a build.
  final Map<Object, (int, int)?> _rates = {};

  (int, int) _rateOf(BridgeLayerEntry entry) {
    final id = entry.layer.internallayerId;
    if (!_rates.containsKey(id)) {
      _rates[id] = null;
      final source = entry.info.source;
      if (source is ItemReference_Footage) {
        source.field0.mediaInfo().then((info) {
          if (!mounted ||
              info == null ||
              info.fpsNum <= 0 ||
              info.fpsDen <= 0) {
            return;
          }
          setState(() => _rates[id] = (info.fpsNum, info.fpsDen));
        });
      }
    }
    return _rates[id] ?? _ui.model.fpsExact;
  }

  BridgeRetimeInterp _interpOf(BridgeLayerEntry entry) {
    final asked = (entry.layer.internallayerId, _ui.model.heldRevision);
    if (asked != _interpAsked || _interp == null) {
      _interpAsked = asked;
      try {
        _interp = entry.layer.getInterpolation();
      } catch (_) {
        _interp = BridgeRetimeInterp.nearest;
      }
    }
    return _interp!;
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
        listenable: _changes,
        builder: (context, _) => _body(context),
      );

  Widget _body(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final targets = retimeTargets(_ui);
    final on = targets.isNotEmpty;
    // Among several, the first one with a Retime is the one to read.
    final retimed = targets.any((e) => e.info.retime != null);
    final first = !on
        ? null
        : targets.firstWhere((e) => e.info.retime != null,
            orElse: () => targets.first);
    final fps = _ui.model.fps;
    final speed =
        _liveSpeed ?? (first == null ? 100.0 : averageSpeed(first, fps));
    final length = first == null ? 0 : first.info.outFrame - first.info.inFrame;
    final label = t.body.copyWith(color: t.textMuted);
    final group = selectedEntries(_ui).length;

    return LayoutBuilder(
      builder: (context, box) {
        final wide = box.maxWidth >= _narrow;
        Widget labelled(String name, Widget control, {double? width}) => Row(
              children: [
                SizedBox(
                  width: width,
                  child: Text(name,
                      style: label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis),
                ),
                const SizedBox(width: 8),
                Expanded(child: control),
              ],
            );
        final speedWell = DragValueField(
          key: const ValueKey('retime-speed'),
          value: speed.abs(),
          min: 1,
          max: 10000,
          decimals: 2,
          suffix: l10n.unitSymbolPercent,
          resetTo: 100,
          onChangeLive: (v) => setState(() => _liveSpeed = v.toDouble()),
          onChangeEnd: (v) {
            setState(() => _liveSpeed = null);
            stretchToSpeed(context, targets, v.toDouble());
          },
          onDragCancel: () => setState(() => _liveSpeed = null),
          onChanged: (v) => stretchToSpeed(context, targets, v.toDouble()),
        );
        final lengthWell = _duration(t, targets, length);

        return GestureDetector(
          key: const ValueKey('retime-panel'),
          behavior: HitTestBehavior.opaque,
          onSecondaryTapDown: on
              ? (d) => showRetimeMenu(context, _ui, targets, d.globalPosition)
              : null,
          child: SizedBox.expand(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(10, 6, 10, 10),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(children: [
                    if (first != null) ...[
                      Container(
                        width: 6,
                        height: 6,
                        decoration: BoxDecoration(
                          color: t.labelColour(first.info.label),
                          shape: BoxShape.circle,
                        ),
                      ),
                      const SizedBox(width: 6),
                    ],
                    Expanded(
                      child: Text(
                        first == null
                            ? ''
                            : group > 1
                                ? '${first.info.name}  +${group - 1}'
                                : first.info.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: t.bodyPrimary,
                      ),
                    ),
                    IgnorePointer(
                      ignoring: !on,
                      child: LumitTooltip(
                        message: retimed
                            ? l10n.menuDisableRetime
                            : l10n.menuEnableRetime,
                        action: 'layer.retime.enable',
                        child: HouseToggle(
                          key: const ValueKey('retime-toggle'),
                          value: retimed,
                          onChanged: (v) => setRetime(context, targets, v),
                        ),
                      ),
                    ),
                  ]),
                  const SizedBox(height: 6),
                  _dimmed(
                    on,
                    Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _strip(
                            t,
                            first,
                            (box.maxHeight - _besideStrip)
                                .clamp(_stripLeast, _stripMost)
                                .toDouble()),
                        const SizedBox(height: 10),
                        if (wide)
                          Row(children: [
                            Expanded(
                                child: labelled(l10n.graphSpeed, speedWell)),
                            const SizedBox(width: 12),
                            Expanded(
                                child: labelled(l10n.duration, lengthWell)),
                          ])
                        else ...[
                          labelled(l10n.graphSpeed, speedWell,
                              width: _labelWidth),
                          const SizedBox(height: 4),
                          labelled(l10n.duration, lengthWell,
                              width: _labelWidth),
                        ],
                        const SizedBox(height: 4),
                        Wrap(
                          spacing: 2,
                          runSpacing: 2,
                          children: [
                            for (final chip in _speedChips)
                              _chip(t, targets, speed, chip),
                          ],
                        ),
                        const SizedBox(height: 10),
                        _commands(targets, speed, retimed),
                        if (first?.info.kind == BridgeLayerKind.footage) ...[
                          const SizedBox(height: 10),
                          Wrap(
                            crossAxisAlignment: WrapCrossAlignment.center,
                            spacing: 8,
                            runSpacing: 3,
                            children: [
                              Text(l10n.sourceInBetweenFrames, style: label),
                              _interpolation(t, targets, first!),
                            ],
                          ),
                        ],
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }

  /// Greyed and out of reach while no layer can use it: the panel holds still
  /// rather than swapping for a caption.
  Widget _dimmed(bool on, Widget child) =>
      on ? child : IgnorePointer(child: Opacity(opacity: 0.4, child: child));

  /// The layer from end to end, its speed drawn along it, with the speed under
  /// the playhead large in the corner and the moment of the source it is showing
  /// opposite. Click or drag to seek. Reads the keys in hand and asks the engine
  /// nothing.
  Widget _strip(LumitTheme t, BridgeLayerEntry? entry, double height) {
    final keys = entry == null ? const <BridgeKeyframe>[] : retimeKeysOf(entry);
    final inFrame = entry?.info.inFrame ?? 0;
    final outFrame = entry?.info.outFrame ?? 1;
    final fps = _ui.model.fps;
    final (fpsNum, fpsDen) =
        entry == null ? _ui.model.fpsExact : _rateOf(entry);
    return LayoutBuilder(
      builder: (context, box) {
        void seek(double x) {
          final share = (x / box.maxWidth).clamp(0.0, 1.0);
          _ui.scrubTo((inFrame + share * (outFrame - inFrame)).round());
        }

        return GestureDetector(
          onTapDown: (d) => seek(d.localPosition.dx),
          onHorizontalDragUpdate: (d) => seek(d.localPosition.dx),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            child: SizedBox(
              height: height,
              child: Stack(
                fit: StackFit.expand,
                children: [
                  CustomPaint(
                    key: const ValueKey('retime-strip'),
                    painter: _SpeedStripPainter(
                      keys: keys,
                      inFrame: inFrame,
                      outFrame: outFrame,
                      fps: fps,
                      playhead: _ui.playheadFrame,
                      ground: t.surface0,
                      fill: t.surface2,
                      line: keys.isEmpty ? t.textMuted : t.textSecondary,
                      normal: t.hairlineStrong,
                      head: t.accent,
                    ),
                  ),
                  ValueListenableBuilder<int>(
                    valueListenable: _ui.playheadFrame,
                    builder: (context, frame, _) {
                      final at = fps <= 0 ? 0.0 : frame / fps;
                      final inside = frame >= inFrame && frame <= outFrame;
                      final speed = !inside
                          ? null
                          : keys.isEmpty
                              ? 100.0
                              : evaluateKeysSpeed(keys, at) * 100;
                      final source = keys.isEmpty || !inside
                          ? null
                          : evaluateKeys(keys, at);
                      return Padding(
                        padding: const EdgeInsets.fromLTRB(6, 2, 6, 0),
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            if (speed != null)
                              Text(
                                '${speedText(speed, fine: false)}'
                                '${l10n.unitSymbolPercent}',
                                key: const ValueKey('retime-readout'),
                                style: t.mono.copyWith(
                                  fontSize: 18,
                                  color: keys.isEmpty
                                      ? t.textMuted
                                      : t.textPrimary,
                                ),
                              ),
                            const Spacer(),
                            if (source != null)
                              Padding(
                                padding: const EdgeInsets.only(top: 3),
                                child: Text(
                                  timecodeOfRateSigned(
                                      (source * fpsNum / fpsDen).round(),
                                      fpsNum,
                                      fpsDen),
                                  key: const ValueKey('retime-source-time'),
                                  style: t.mono.copyWith(
                                      fontSize: wellTextSize,
                                      color: t.textMuted),
                                ),
                              ),
                          ],
                        ),
                      );
                    },
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _duration(LumitTheme t, List<BridgeLayerEntry> targets, int length) {
    final (fpsNum, fpsDen) = _ui.model.fpsExact;
    return TimeReadout(
      key: const ValueKey('retime-duration'),
      frame: length,
      format: (f) => timecodeOfRateSigned(f, fpsNum, fpsDen),
      parse: (text) => framesOfTimecodeSigned(text, fpsNum, fpsDen),
      widthChars: timecodeChars(fpsNum, fpsDen),
      style: t.mono.copyWith(fontSize: wellTextSize),
      well: true,
      minFrame: 1,
      maxFrame: 1000000,
      draggable: true,
      onCommit: (f) => stretchToLength(context, targets, f),
    );
  }

  Widget _chip(
      LumitTheme t, List<BridgeLayerEntry> targets, double speed, int percent) {
    final current = (speed.abs() - percent).abs() < 0.01;
    return HouseButton(
      key: ValueKey<String>('retime-chip-$percent'),
      small: true,
      frameless: !current,
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
      onPressed: targets.isEmpty
          ? null
          : () => stretchToSpeed(context, targets, percent.toDouble()),
      child: Text('$percent',
          style: t.mono.copyWith(
              fontSize: wellTextSize,
              color: current ? t.textPrimary : t.textSecondary)),
    );
  }

  Widget _commands(
          List<BridgeLayerEntry> targets, double speed, bool retimed) =>
      Wrap(
        spacing: 4,
        runSpacing: 4,
        children: [
          ValueListenableBuilder<int>(
            valueListenable: _ui.playheadFrame,
            builder: (context, frame, _) => HouseButton(
              key: const ValueKey('retime-freeze'),
              small: true,
              onPressed: targets.any((e) => canFreezeAt(e, frame))
                  ? () => freezeAtPlayhead(context, targets)
                  : null,
              child: Text(l10n.retimeFreeze),
            ),
          ),
          HouseButton(
            key: const ValueKey('retime-reverse'),
            small: true,
            active: speed < 0,
            onPressed:
                targets.isEmpty ? null : () => reverseLayers(context, targets),
            child: Text(l10n.easingReverse),
          ),
          HouseButton(
            key: const ValueKey('retime-reset'),
            small: true,
            onPressed: retimed ? () => resetSpeed(context, targets) : null,
            child: Text(l10n.clipResetSpeed),
          ),
        ],
      );

  Widget _interpolation(
      LumitTheme t, List<BridgeLayerEntry> targets, BridgeLayerEntry first) {
    final now = _interpOf(first);
    return Segment(
      key: const ValueKey('retime-interp'),
      children: [
        for (final interp in BridgeRetimeInterp.values)
          SegmentOption(
            key: ValueKey<String>('retime-interp-${interp.name}'),
            active: interp == now,
            onTap: () {
              _interpAsked = null;
              setInterpolation(context, targets, interp);
            },
            child: Text(t.kickerCase(interpLabel(interp)),
                style: interp == now ? t.kickerOn : t.kicker),
          ),
      ],
    );
  }
}

/// The layer's speed drawn from its in point to its out point: filled up from
/// nothing to the line, with a rule at 100%, so a slow stretch is low, a freeze
/// is empty and a fast one stands above the rule.
class _SpeedStripPainter extends CustomPainter {
  final List<BridgeKeyframe> keys;
  final int inFrame, outFrame;
  final double fps;
  final ValueNotifier<int> playhead;
  final Color ground, fill, line, normal, head;

  _SpeedStripPainter({
    required this.keys,
    required this.inFrame,
    required this.outFrame,
    required this.fps,
    required this.playhead,
    required this.ground,
    required this.fill,
    required this.line,
    required this.normal,
    required this.head,
  }) : super(repaint: playhead);

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = ground);
    final span = outFrame - inFrame;
    if (span <= 0 || fps <= 0 || size.width <= 0) return;
    double speedAt(double x) => keys.isEmpty
        ? 100
        : envelopeSpeedAt(keys, (inFrame + x / size.width * span) / fps);
    const step = 2.0;
    var lo = 0.0, hi = 110.0;
    for (var x = 0.0; x <= size.width; x += step) {
      final s = speedAt(x);
      if (s < lo) lo = s;
      if (s > hi) hi = s;
    }
    // The readout has the top of the strip to itself.
    const headroom = 24.0;
    final room = size.height - headroom;
    double yOf(double s) => size.height - (s - lo) / (hi - lo) * room;
    final zero = yOf(0);
    final curve = Path()..moveTo(0, yOf(speedAt(0)));
    for (var x = step; x <= size.width; x += step) {
      curve.lineTo(x, yOf(speedAt(x)));
    }
    final area = Path.from(curve)
      ..lineTo(size.width, zero)
      ..lineTo(0, zero)
      ..close();
    canvas.drawPath(area, Paint()..color = fill);
    final rule = yOf(100);
    canvas.drawLine(
        Offset(0, rule), Offset(size.width, rule), Paint()..color = normal);
    canvas.drawPath(
        curve,
        Paint()
          ..color = line
          ..strokeWidth = 1.2
          ..style = PaintingStyle.stroke);
    final frame = playhead.value;
    if (frame >= inFrame && frame <= outFrame) {
      final x = (frame - inFrame) / span * size.width;
      canvas.drawLine(
          Offset(x, 0), Offset(x, size.height), Paint()..color = head);
    }
  }

  @override
  bool shouldRepaint(_SpeedStripPainter old) =>
      old.keys != keys ||
      old.inFrame != inFrame ||
      old.outFrame != outFrame ||
      old.fps != fps ||
      old.line != line ||
      old.fill != fill ||
      old.ground != ground;
}
