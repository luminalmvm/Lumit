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

/// What the speed and the source time need side by side on the strip.
const double _sourceTimeRoom = 160;

class RetimePanelFrb extends StatefulWidget {
  const RetimePanelFrb({super.key});

  @override
  State<RetimePanelFrb> createState() => _RetimePanelFrbState();
}

class _RetimePanelFrbState extends State<RetimePanelFrb> {
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);
  late final Listenable _changes =
      Listenable.merge([_ui.model, _ui.selectedLayers, _ui.cutClips]);

  /// The speed a drag on the well is asking for, shown while it runs and
  /// written once when it ends.
  double? _liveSpeed;

  /// How the first target finds its in-between frames. The read model only
  /// says whether flow is on, so this is asked once per document revision.
  BridgeRetimeInterp _interp = BridgeRetimeInterp.nearest;
  Object? _interpAsked;

  /// The footage's own rate by layer id, which the source clock counts in, as
  /// the Retime row's does. Asked once per layer.
  final Map<Object, (int, int)?> _rates = {};

  (int, int) _rateOf(BridgeLayerEntry entry) =>
      _rates[entry.layer.internallayerId] ?? _ui.model.fpsExact;

  /// Among several, the first one with a Retime is the one to read.
  BridgeLayerEntry? _firstOf(List<BridgeLayerEntry> targets) => targets.isEmpty
      ? null
      : targets.firstWhere((e) => e.info.retime != null,
          orElse: () => targets.first);

  /// Ask the engine what the read model doesn't hold, when the selection or the
  /// document changes, so a build only ever reads the answers.
  void _askEngine() {
    final first = _firstOf(retimeTargets(_ui));
    if (first == null) return;
    final id = first.layer.internallayerId;
    final source = first.info.source;
    if (!_rates.containsKey(id)) {
      _rates[id] = null;
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
    final asked = (id, _ui.model.heldRevision);
    if (asked == _interpAsked) return;
    _interpAsked = asked;
    try {
      _interp = first.layer.getInterpolation();
    } catch (_) {
      _interp = BridgeRetimeInterp.nearest;
    }
  }

  @override
  void initState() {
    super.initState();
    _changes.addListener(_askEngine);
    _askEngine();
  }

  @override
  void dispose() {
    _changes.removeListener(_askEngine);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
        listenable: _changes,
        builder: (context, _) => _body(context),
      );

  Widget _body(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // A picked clip is the subject where there is one, and its layer then has
    // no Retime of its own to read.
    final clips = retimeClips(_ui);
    final clip = clips.isEmpty ? null : clips.first;
    final targets = clip != null ? const <BridgeLayerEntry>[] : retimeTargets(_ui);
    final on = targets.isNotEmpty || clip != null;
    final retimed =
        clip?.clip.retimed ?? targets.any((e) => e.info.retime != null);
    final first = _firstOf(targets);
    final fps = _ui.model.fps;
    final speed = _liveSpeed ??
        (clip != null
            ? clipAverageSpeed(clip.clip, fps)
            : first == null
                ? 100.0
                : averageSpeed(first, fps));
    final length = clip != null
        ? (clip.clip.endFrame - clip.clip.startFrame).toInt()
        : first == null
            ? 0
            : first.info.outFrame - first.info.inFrame;
    final label = t.body.copyWith(color: t.textMuted);
    final group = clip != null ? clips.length : selectedEntries(_ui).length;
    final name = clip?.clip.sourceName ?? first?.info.name ?? '';
    final tint = clip != null
        ? clip.clip.label ??
            _ui.model.byId(clip.layer.internallayerId)?.info.label
        : first?.info.label;
    void toSpeed(double percent) => clip != null
        ? setClipsSpeed(context, clips, percent)
        : stretchToSpeed(context, targets, percent);

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
            toSpeed(v.toDouble());
          },
          onDragCancel: () => setState(() => _liveSpeed = null),
          onChanged: (v) => toSpeed(v.toDouble()),
        );
        final lengthWell = _duration(t, targets, clips, length);

        return GestureDetector(
          key: const ValueKey('retime-panel'),
          behavior: HitTestBehavior.opaque,
          onSecondaryTapDown: clip != null
              ? (d) => showClipRetimeMenu(context, _ui, clips, d.globalPosition)
              : targets.isNotEmpty
                  ? (d) =>
                      showRetimeMenu(context, _ui, targets, d.globalPosition)
                  : null,
          child: SizedBox.expand(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(10, 6, 10, 10),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(children: [
                    if (tint != null) ...[
                      Container(
                        width: 6,
                        height: 6,
                        decoration: BoxDecoration(
                          color: t.labelColour(tint),
                          shape: BoxShape.circle,
                        ),
                      ),
                      const SizedBox(width: 6),
                    ],
                    Expanded(
                      child: Text(
                        name.isEmpty
                            ? ''
                            : group > 1
                                ? '$name  +${group - 1}'
                                : name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: t.bodyPrimary,
                      ),
                    ),
                    // A clip always has its line, so there is nothing to turn on.
                    if (clip == null)
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
                        // Flat at 100% until there's a Retime to draw, so it
                        // reserves no ground for a shape it hasn't got.
                        _strip(
                            t,
                            first,
                            clip?.clip,
                            retimed
                                ? (box.maxHeight - _besideStrip)
                                    .clamp(_stripLeast, _stripMost)
                                    .toDouble()
                                : _stripLeast),
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
                              _chip(t, on, toSpeed, speed, chip),
                          ],
                        ),
                        const SizedBox(height: 10),
                        _commands(t, targets, clips, speed, retimed),
                        // A clip follows its layer here.
                        if (clip == null &&
                            first?.info.kind == BridgeLayerKind.footage) ...[
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
  /// the playhead and the moment of the source it's showing beside the playhead
  /// line, so they read as "here". Past the middle they hang on the line's left.
  /// Click or drag to seek. Reads the keys in hand and asks the engine nothing.
  Widget _strip(
      LumitTheme t, BridgeLayerEntry? entry, BridgeClip? clip, double height) {
    final keys = clip != null
        ? keysOf(clip.retime)
        : entry == null
            ? const <BridgeKeyframe>[]
            : retimeKeysOf(entry);
    final inFrame = clip?.startFrame.toInt() ?? entry?.info.inFrame ?? 0;
    final outFrame = clip?.endFrame.toInt() ?? entry?.info.outFrame ?? 1;
    final fps = _ui.model.fps;
    // A clip's keys are on its own clock, and a layer's on the composition's.
    final zero = clip == null || fps <= 0 ? 0.0 : inFrame / fps;
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
                      clock: zero,
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
                      final at = (fps <= 0 ? 0.0 : frame / fps) - zero;
                      final inside = frame >= inFrame && frame <= outFrame;
                      final speed = !inside
                          ? null
                          : keys.isEmpty
                              ? 100.0
                              : evaluateKeysSpeed(keys, at) * 100;
                      if (speed == null) return const SizedBox.shrink();
                      final source = keys.isEmpty || !inside
                          ? null
                          : evaluateKeys(keys, at);
                      final span = outFrame - inFrame;
                      final x = span <= 0
                          ? 0.0
                          : (frame - inFrame) / span * box.maxWidth;
                      final hangs = x > box.maxWidth / 2;
                      final readout = Text(
                        '${speedText(speed, fine: false)}'
                        '${l10n.unitSymbolPercent}',
                        key: const ValueKey('retime-readout'),
                        maxLines: 1,
                        style: t.mono.copyWith(
                          fontSize: 18,
                          color: keys.isEmpty ? t.textMuted : t.textPrimary,
                        ),
                      );
                      // Half the strip is the least the pair ever has to stand
                      // in, so the source time goes first when that's short.
                      final time = source == null ||
                              box.maxWidth < _sourceTimeRoom * 2
                          ? null
                          : Text(
                              timecodeOfRateSigned(
                                  (source * fpsNum / fpsDen).round(),
                                  fpsNum,
                                  fpsDen),
                              key: const ValueKey('retime-source-time'),
                              maxLines: 1,
                              style: t.mono.copyWith(
                                  fontSize: wellTextSize, color: t.textMuted),
                            );
                      return Positioned(
                        top: 2,
                        left: hangs ? null : x + 6,
                        right: hangs ? box.maxWidth - x + 6 : null,
                        child: Row(
                          mainAxisSize: MainAxisSize.min,
                          crossAxisAlignment: CrossAxisAlignment.baseline,
                          textBaseline: TextBaseline.alphabetic,
                          children: hangs
                              ? [
                                  if (time != null) ...[
                                    time,
                                    const SizedBox(width: 8),
                                  ],
                                  readout,
                                ]
                              : [
                                  readout,
                                  if (time != null) ...[
                                    const SizedBox(width: 8),
                                    time,
                                  ],
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

  Widget _duration(LumitTheme t, List<BridgeLayerEntry> targets,
      List<RetimeClip> clips, int length) {
    final (fpsNum, fpsDen) = _ui.model.fpsExact;
    return TimeReadout(
      key: const ValueKey('retime-duration'),
      frame: length,
      format: (f) => timecodeOfRateSigned(f, fpsNum, fpsDen),
      parse: (text) => framesOfTimecodeSigned(text, fpsNum, fpsDen),
      widthChars: timecodeChars(fpsNum, fpsDen),
      style: t.mono.copyWith(fontSize: wellTextSize),
      well: true,
      wellFaceHeight: wellHeight,
      minFrame: 1,
      maxFrame: 1000000,
      draggable: true,
      onCommit: (f) => clips.isNotEmpty
          ? stretchClipsToLength(context, clips, f)
          : stretchToLength(context, targets, f),
    );
  }

  Widget _chip(LumitTheme t, bool on, void Function(double) toSpeed,
      double speed, int percent) {
    final current = (speed.abs() - percent).abs() < 0.01;
    return HouseButton(
      key: ValueKey<String>('retime-chip-$percent'),
      small: true,
      frameless: !current,
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
      onPressed: on ? () => toSpeed(percent.toDouble()) : null,
      child: Text('$percent',
          style: t.mono.copyWith(
              fontSize: wellTextSize,
              color: current ? t.textPrimary : t.textSecondary)),
    );
  }

  /// How long a freeze holds, beside its button: the length last set, for a
  /// layer or for a clip, each its own.
  Widget _holdWell(LumitTheme t, bool onClip, bool live) {
    final settings = _ui.workspace.interface;
    final (fpsNum, fpsDen) = _ui.model.fpsExact;
    final fps = _ui.model.fps <= 0 ? 1.0 : _ui.model.fps;
    final seconds =
        onClip ? settings.cutFreezeSeconds : settings.retimeHoldSeconds;
    return SizedBox(
      width: 100,
      child: IgnorePointer(
        ignoring: !live,
        child: LumitTooltip(
          message: l10n.duration,
          child: TimeReadout(
          key: const ValueKey('retime-hold'),
          frame: (seconds * fps).round(),
          format: (f) => timecodeOfRateSigned(f, fpsNum, fpsDen),
          parse: (text) => framesOfTimecodeSigned(text, fpsNum, fpsDen),
          widthChars: timecodeChars(fpsNum, fpsDen),
          style: t.mono.copyWith(fontSize: wellTextSize),
          well: true,
          wellFaceHeight: wellHeight,
          minFrame: 1,
          maxFrame: 1000000,
          draggable: true,
          onCommit: (f) => setState(() {
            if (onClip) {
              settings.cutFreezeSeconds = f / fps;
            } else {
              settings.retimeHoldSeconds = f / fps;
            }
            _ui.workspace.settingsChanged();
          }),
          ),
        ),
      ),
    );
  }

  Widget _commands(LumitTheme t, List<BridgeLayerEntry> targets,
      List<RetimeClip> clips, double speed, bool retimed) {
    final onClip = clips.isNotEmpty;
    bool insideClip(int frame) =>
        clips.any((c) => frame >= c.clip.startFrame && frame < c.clip.endFrame);
    bool insideLayer(int frame) =>
        targets.any((e) => frame >= e.info.inFrame && frame < e.info.outFrame);
    return Wrap(
      spacing: 4,
      runSpacing: 4,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        ValueListenableBuilder<int>(
          valueListenable: _ui.playheadFrame,
          builder: (context, frame, _) {
            final canFreeze = onClip
                ? insideClip(frame)
                : targets.any((e) => canFreezeAt(e, frame));
            return Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                HouseButton(
                  key: const ValueKey('retime-freeze'),
                  small: true,
                  onPressed: canFreeze
                      ? () => onClip
                          ? freezeClips(context, clips)
                          : freezeAtPlayhead(context, targets)
                      : null,
                  child: Text(l10n.retimeFreeze),
                ),
                const SizedBox(width: 2),
                _holdWell(t, onClip, targets.isNotEmpty || onClip),
              ],
            );
          },
        ),
        ValueListenableBuilder<int>(
          valueListenable: _ui.playheadFrame,
          builder: (context, frame, _) => Builder(
            builder: (button) => HouseButton(
              key: const ValueKey('retime-ramp'),
              small: true,
              onPressed: (onClip ? insideClip(frame) : insideLayer(frame))
                  ? () {
                      final box = button.findRenderObject() as RenderBox;
                      askToRamp(context, targets, clips,
                          box.localToGlobal(Offset(0, box.size.height + 2)));
                    }
                  : null,
              child: Text(l10n.retimeRamp),
            ),
          ),
        ),
        HouseButton(
          key: const ValueKey('retime-reverse'),
          small: true,
          active: speed < 0,
          onPressed: onClip
              ? () => reverseClips(context, clips)
              : targets.isEmpty
                  ? null
                  : () => reverseLayers(context, targets),
          child: Text(l10n.easingReverse),
        ),
        HouseButton(
          key: const ValueKey('retime-reset'),
          small: true,
          onPressed: !retimed
              ? null
              : onClip
                  ? () => resetClips(context, clips)
                  : () => resetSpeed(context, targets),
          child: Text(l10n.clipResetSpeed),
        ),
      ],
    );
  }

  Widget _interpolation(
      LumitTheme t, List<BridgeLayerEntry> targets, BridgeLayerEntry first) {
    final now = _interp;
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

  /// When the keys' clock reads nought, in seconds on the composition's.
  final double clock;
  final int inFrame, outFrame;
  final double fps;
  final ValueNotifier<int> playhead;
  final Color ground, fill, line, normal, head;

  _SpeedStripPainter({
    required this.keys,
    required this.clock,
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
        : envelopeSpeedAt(keys, (inFrame + x / size.width * span) / fps - clock);
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
      old.clock != clock ||
      old.inFrame != inFrame ||
      old.outFrame != outFrame ||
      old.fps != fps ||
      old.line != line ||
      old.fill != fill ||
      old.ground != ground;
}
