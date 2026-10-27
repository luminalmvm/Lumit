// The Mixer panel (docs/09 §3.1, the approved AudioWorkspace
// board): one strip per audible row of the fronted comp, in comp order, and
// the Master strip at the right-hand edge.
//
// Each strip is the board's drawing: the layer's name underlined in its label
// colour, a pan pot, a fader beside the stereo meters, the dB well, and the
// mute and solo switches. The Master strip swaps the pot for the limiter lamp
// and carries the muted LUFS placeholder (loudness measurement is post-v1,
// docs/09 §8).
//
// **The meters run off the feed.** The rest moves only as the house controls
// do: a picked strip's fill and a switch's light ease in. One
// [AudioMeterFeed] polls the engine's tap and the meter *painters* listen to
// its frame directly, each
// inside its own RepaintBoundary — a tick repaints the bars and rebuilds no
// widget at all, which is what keeps the redraw gates green
// (docs/impl/ui-performance.md WP-2). Everything else draws from the held comp
// model and rebuilds only when the document moves.

import 'dart:math' as math;

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'audio_meters_feed.dart';
import 'keyframe_controls_frb.dart';
import 'placeholder.dart';
import 'timeline_extras_frb.dart' show rowSelectionFill;

/// The fader's floor: the engine's own knee, at or below which the sound is
/// exactly off (`lumit_audio::mix::VOLUME_FLOOR_DB`).
const double _faderFloorDb = -100.0;

/// The fader's taper, as a console prints it: points of travel against dB,
/// silence at the bottom, unity three quarters of the way up and +6 at the
/// top, with most of the throw spent near unity. The +12 the value field
/// allows is still reachable by typing in the Audio panel.
const List<(double, double)> _taper = [
  (0.0, _faderFloorDb),
  (0.05, -60),
  (0.15, -40),
  (0.25, -30),
  (0.375, -20),
  (0.55, -10),
  (0.75, 0),
  (1.0, 6),
];

/// Where [db] sits on the fader's travel, nought to one.
double _faderFraction(double db) {
  for (var i = 1; i < _taper.length; i++) {
    final (f0, d0) = _taper[i - 1];
    final (f1, d1) = _taper[i];
    if (db <= d1) {
      return (f0 + (db - d0) / (d1 - d0) * (f1 - f0)).clamp(0.0, 1.0);
    }
  }
  return 1;
}

/// The dB a point of the fader's travel reads.
double _faderDb(double fraction) {
  final f = fraction.clamp(0.0, 1.0);
  for (var i = 1; i < _taper.length; i++) {
    final (f0, d0) = _taper[i - 1];
    final (f1, d1) = _taper[i];
    if (f <= f1) return d0 + (f - f0) / (f1 - f0) * (d1 - d0);
  }
  return _taper.last.$2;
}

class MixerPanelFrb extends StatefulWidget {
  /// A test's own feed, silent and hand-pulsed. The application passes none
  /// and the panel makes its own polling one.
  final AudioMeterFeed? feed;

  const MixerPanelFrb({super.key, this.feed});

  @override
  State<MixerPanelFrb> createState() => _MixerPanelFrbState();
}

class _MixerPanelFrbState extends State<MixerPanelFrb> {
  late final AudioMeterFeed _feed = widget.feed ?? AudioMeterFeed();

  /// Which layers can make a sound, probed once per layer and remembered,
  /// asked exactly as the Timeline asks it.
  final Map<String, bool> _hasAudio = {};

  /// The layers the panel was opened on, and those added since. A strip for
  /// one added since comes up into its place; the desk the panel opened on is
  /// simply there.
  Set<String>? _opening;
  final Set<String> _added = {};

  @override
  void initState() {
    super.initState();
    if (widget.feed == null) _feed.start();
  }

  /// The strips' own scroll, for the bar under them and the wheel.
  final ScrollController _strips = ScrollController();

  @override
  void dispose() {
    if (widget.feed == null) _feed.dispose();
    _strips.dispose();
    super.dispose();
  }

  /// A plain wheel turn carries the strips sideways: the list only runs one
  /// way, so there is nothing else for it to mean.
  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent || !_strips.hasClients) return;
    final by = event.scrollDelta.dx != 0
        ? event.scrollDelta.dx
        : event.scrollDelta.dy;
    final p = _strips.position;
    _strips.jumpTo((p.pixels + by).clamp(0.0, p.maxScrollExtent));
  }

  /// Fill in any missing has-audio answers, off the build (the probe opens
  /// files). Claiming the slot first keeps a rebuild mid-probe from asking
  /// twice.
  void _refreshAudio(List<BridgeLayerEntry> layers) {
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      if (_hasAudio.containsKey(id)) continue;
      _hasAudio[id] = false;
      entry.layer.hasAudio().then((has) {
        if (!mounted || _hasAudio[id] == has) return;
        setState(() => _hasAudio[id] = has);
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    final comp = ui.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.audio,
        title: l10n.panelMixer,
        hint: l10n.selectACompositionFirst,
      );
    }
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final layers = ui.model.heldLayers;
        _refreshAudio(layers);
        final ids = {
          for (final entry in layers) entry.layer.internallayerId.toString(),
        };
        final opening = _opening ??= ids;
        _added.addAll(ids.difference(opening));
        final reveal = ThemeScope.of(context).motion;
        final sounding = [
          for (final entry in layers)
            if (_hasAudio[entry.layer.internallayerId.toString()] ?? false)
              entry,
        ];
        return Container(
          color: t.surface0,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(
                child: sounding.isEmpty
                    ? Center(
                        child: Padding(
                          padding: const EdgeInsets.all(12),
                          child: Text(
                            l10n.mixerNoSound,
                            style: t.small,
                            textAlign: TextAlign.center,
                          ),
                        ),
                      )
                    // A bar under the strips says there are more than the
                    // panel shows, and where they are.
                    : Listener(
                        onPointerSignal: _wheel,
                        child: HouseScrollbar(
                        controller: _strips,
                        crossAxisMargin: 2,
                        child: ListView(
                        controller: _strips,
                        scrollDirection: Axis.horizontal,
                        children: [
                          for (final entry in sounding)
                            Entrance(
                              key: ValueKey(
                                  'strip-in-${entry.layer.internallayerId}'),
                              spec: _added.contains(
                                      entry.layer.internallayerId.toString())
                                  ? reveal.reveal
                                  : MotionSpec.still,
                              rise: reveal.revealRise,
                              child: _MixerStrip(
                                key: ValueKey(
                                    'strip-${entry.layer.internallayerId}'),
                                comp: comp,
                                entry: entry,
                                feed: _feed,
                                onChanged: () {
                                  ui.model.refresh();
                                  comp.audioPrepare();
                                },
                              ),
                            ),
                        ],
                      ),
                      ),
                      ),
              ),
              _MasterStrip(comp: comp, feed: _feed),
            ],
          ),
        );
      },
    );
  }
}

/// The width the board draws a strip at.
const double _stripWidth = 62;

class _MixerStrip extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;
  final AudioMeterFeed feed;
  final VoidCallback onChanged;

  const _MixerStrip({
    super.key,
    required this.comp,
    required this.entry,
    required this.feed,
    required this.onChanged,
  });

  @override
  State<_MixerStrip> createState() => _MixerStripState();
}

class _MixerStripState extends State<_MixerStrip> {
  /// Mid-drag values, shown live and committed once on release so the whole
  /// gesture is one undo step (the value fields' own discipline).
  double? _dragDb;
  double? _dragPan;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final info = widget.entry.info;
    final layer = widget.entry.layer;
    final id = layer.internallayerId.toString();
    final switches = info.switches;

    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    // Keyframed Volume or Pan reads at the playhead, exactly as the Timeline's
    // own rows do — only then does the strip listen to it. A strip whose sound
    // controls are static cannot change with the playhead, so it never
    // rebuilds for one.
    final animated = info.volumeDb is! BridgeScalar_Static ||
        info.pan is! BridgeScalar_Static;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final mark = ThemeScope.of(context).motion.mark;

    // A press anywhere on the strip picks its layer, on the way down so the
    // pick lands before the fader or the pot starts its own gesture. The
    // picked strip wears the Timeline's selection fill.
    return Listener(
      onPointerDown: (_) {
        if (!ui.selectedLayerIds.contains(layer.internallayerId)) {
          ui.setSelection([layer]);
        }
      },
      child: ValueListenableBuilder<List<LayerReference>>(
        valueListenable: ui.selectedLayers,
        builder: (context, picked, child) => AnimatedContainer(
          duration: mark.duration,
          curve: mark.curve,
          width: _stripWidth,
          decoration: BoxDecoration(
            color: picked.any((l) => l.internallayerId == layer.internallayerId)
                ? rowSelectionFill(t)
                : rowSelectionFill(t).withValues(alpha: 0),
            border: Border(right: BorderSide(color: t.hairline)),
          ),
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: child,
        ),
        child: !animated
            ? _body(context, t, info, layer, id, switches, playhead.value)
            : ValueListenableBuilder<int>(
                valueListenable: playhead,
                builder: (context, frame, _) =>
                    _body(context, t, info, layer, id, switches, frame),
              ),
      ),
    );
  }

  Widget _body(
    BuildContext context,
    LumitTheme t,
    BridgeLayerInfo info,
    LayerReference layer,
    String id,
    BridgeLayerSwitches switches,
    int frame,
  ) {
    final volume = info.volumeDb;
    final pan = info.pan;
    final db = _dragDb ?? _sampled(volume, frame);
    final panValue = _dragPan ?? _sampled(pan, frame);
    return Column(
            children: [
              LumitTooltip(
                message: info.name,
                child: Container(
                  constraints: const BoxConstraints(maxWidth: _stripWidth - 8),
                  decoration: BoxDecoration(
                    border: Border(
                      bottom: BorderSide(
                        color: t.labelColour(info.label),
                        width: 2,
                      ),
                    ),
                  ),
                  padding: const EdgeInsets.only(bottom: 2),
                  child: Text(
                    info.name,
                    style: t.mono.copyWith(fontSize: 9, color: t.textPrimary),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ),
              const SizedBox(height: 6),
              // The chain indicator (AP5, the board's small mark): the strip
              // says the layer's sound goes through plugins without listing
              // them — the stack is the rack, and Effect controls is where the
              // rack is worked. Enabled entries only, because a fully bypassed
              // chain is not in the sound.
              if (info.effects
                      .where((e) => e.enabled && e.audio)
                      .length
                  case final chain when chain > 0) ...[
                LumitTooltip(
                  message: l10n.mixerChain(chain),
                  child: Container(
                    key: ValueKey('chain-$id'),
                    padding:
                        const EdgeInsets.symmetric(horizontal: 4),
                    decoration: BoxDecoration(
                      border: Border.all(color: t.hairlineStrong),
                      borderRadius: BorderRadius.circular(2),
                    ),
                    child: Text(
                      'FX $chain',
                      style: t.mono
                          .copyWith(fontSize: 9, color: t.textMuted),
                    ),
                  ),
                ),
                const SizedBox(height: 4),
              ],
              PanPot(
                key: ValueKey('pot-$id'),
                value: panValue,
                onLive: (v) => setState(() => _dragPan = v),
                onCommit: (v) => _commitPan(pan, v, frame),
                onCancel: () => setState(() => _dragPan = null),
              ),
              const SizedBox(height: 4),
              Expanded(
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _Fader(
                      key: ValueKey('fader-$id'),
                      db: db,
                      onLive: (v) => setState(() => _dragDb = v),
                      onCommit: (v) => _commitVolume(volume, v, frame),
                      onCancel: () => setState(() => _dragDb = null),
                    ),
                    const SizedBox(width: 3),
                    MeterBand(stripId: id, feed: widget.feed),
                  ],
                ),
              ),
              const SizedBox(height: 4),
              _DbWell(db: db),
              const SizedBox(height: 4),
              Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  _SwitchGlyph(
                    keyName: 'mute-$id',
                    mark: switches.audible ? LumitIcons.audio : LumitIcons.muted,
                    on: switches.audible,
                    tip: switches.audible
                        ? l10n.switchAudible
                        : l10n.switchMuted,
                    onTap: () {
                      layer.setSwitch(
                          switch_: BridgeLayerSwitch.audible,
                          on_: !switches.audible);
                      widget.onChanged();
                    },
                  ),
                  const SizedBox(width: 5),
                  _SoloChip(
                    keyName: 'solo-$id',
                    on: switches.solo,
                    onTap: () {
                      layer.setSwitch(
                          switch_: BridgeLayerSwitch.solo, on_: !switches.solo);
                      widget.onChanged();
                    },
                  ),
                ],
              ),
            ],
          );
  }

  double _sampled(BridgeScalar scalar, int frame) =>
      scalar is BridgeScalar_Static
          ? scalar.field0
          : sampledScalar(scalar, timeOfFrame(widget.comp, frame));

  /// One commit on release: the key under the playhead updated (or planted)
  /// when the property is animated, the plain value when it is not — the same
  /// door the Timeline's Volume row writes through.
  void _commitVolume(BridgeScalar scalar, double value, int frame) {
    widget.entry.layer.setVolumeDb(
      value: scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    setState(() => _dragDb = null);
    widget.onChanged();
  }

  void _commitPan(BridgeScalar scalar, double value, int frame) {
    widget.entry.layer.setPan(
      value: scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    setState(() => _dragPan = null);
    widget.onChanged();
  }
}

/// The Master strip: the fader is a gain stage on the sum, the lamp is
/// the limiter's sticky clip light, and the LUFS readout is the muted
/// placeholder the board draws — loudness measurement is post-v1 (docs/09 §8).
class _MasterStrip extends StatefulWidget {
  final CompositionReference comp;
  final AudioMeterFeed feed;

  const _MasterStrip({required this.comp, required this.feed});

  @override
  State<_MasterStrip> createState() => _MasterStripState();
}

class _MasterStripState extends State<_MasterStrip> {
  double? _dragDb;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final db = _dragDb ?? widget.comp.masterVolumeDb();
    return Container(
      width: _stripWidth,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Column(
        children: [
          Text(
            l10n.mixerMaster,
            style: t.small.copyWith(color: t.textPrimary),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          const SizedBox(height: 6),
          _LimiterLamp(feed: widget.feed),
          const SizedBox(height: 4),
          Expanded(
            child: Row(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _Fader(
                  key: const ValueKey('fader-master'),
                  db: db,
                  onLive: (v) => setState(() => _dragDb = v),
                  onCommit: (v) {
                    widget.comp.setMasterVolumeDb(db: v);
                    setState(() => _dragDb = null);
                    context.read<LumitUiState>().model.refresh();
                    widget.comp.audioPrepare();
                  },
                  onCancel: () => setState(() => _dragDb = null),
                ),
                const SizedBox(width: 3),
                MeterBand(stripId: '', feed: widget.feed),
              ],
            ),
          ),
          const SizedBox(height: 4),
          _DbWell(db: db),
          const SizedBox(height: 4),
          // The muted LUFS placeholder: measurement is a planned export
          // option, and a number invented here would be a lie with decimals.
          Text(
            l10n.mixerLufsPending,
            style: t.caption.copyWith(color: t.textDisabled),
          ),
        ],
      ),
    );
  }
}

/// The limiter's lamp: LIM, lit while the sticky clip flag is
/// up. Clicking it is the desk's "I have seen it" — the lights go out, the mix
/// is untouched. Its own tiny widget under the clip listenable, so a tick of
/// the meters never rebuilds it and a clip lights it without touching the
/// strips.
class _LimiterLamp extends StatelessWidget {
  final AudioMeterFeed feed;

  const _LimiterLamp({required this.feed});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ValueListenableBuilder<bool>(
      valueListenable: feed.clipped,
      builder: (context, lit, _) => LumitTooltip(
        message: lit ? l10n.mixerLimiterClipped : l10n.mixerLimiter,
        child: HoverMark(
          builder: (context, over) => GestureDetector(
            key: const ValueKey('limiter-lamp'),
            onTap: resetAudioClip,
            child: AnimatedContainer(
              duration: ThemeScope.of(context).motion.mark.duration,
              padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
              decoration: BoxDecoration(
                color: t.error.withValues(alpha: lit ? 0.18 : 0),
                border: Border.all(color: lit ? t.error : t.hairlineStrong),
                borderRadius: BorderRadius.circular(2),
              ),
              child: Text(
                'LIM',
                style: t.mono.copyWith(
                  fontSize: 9,
                  color: Color.lerp(lit ? t.error : t.textMuted,
                      t.textPrimary, 0.35 * over),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// One strip's stereo meters: RMS body, peak cap, and the panel's own hold
/// line, on the fader's scale beside them.
///
/// A [RepaintBoundary] around a [CustomPaint] whose painter repaints off the
/// feed's frame notifier directly — a meter tick is a repaint of this band
/// and a rebuild of nothing.
class MeterBand extends StatelessWidget {
  final String stripId;
  final AudioMeterFeed feed;

  const MeterBand({super.key, required this.stripId, required this.feed});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return RepaintBoundary(
      key: ValueKey('meter-band-$stripId'),
      child: SizedBox(
        width: 11,
        child: CustomPaint(
          painter: MeterPainter(
            frame: feed.frame,
            stripId: stripId,
            trough: t.surface0,
            edge: t.hairline,
            body: t.success,
            cap: t.warning,
            hold: t.textPrimary,
          ),
        ),
      ),
    );
  }
}

class MeterPainter extends CustomPainter {
  final ValueListenable<AudioMeterFrame> frame;
  final String stripId;
  final Color trough, edge, body, cap, hold;

  MeterPainter({
    required this.frame,
    required this.stripId,
    required this.trough,
    required this.edge,
    required this.body,
    required this.cap,
    required this.hold,
  }) : super(repaint: frame);

  @override
  void paint(Canvas canvas, Size size) {
    final levels = frame.value.of(stripId);
    const barWidth = 4.0;
    const gap = 3.0;
    final left = (size.width - barWidth * 2 - gap) / 2;
    _bar(canvas, size, left, levels.rmsLeft, levels.peakLeft, levels.holdLeft);
    _bar(canvas, size, left + barWidth + gap, levels.rmsRight,
        levels.peakRight, levels.holdRight);
  }

  void _bar(Canvas canvas, Size size, double x, double rms, double peak,
      double holdValue) {
    const barWidth = 4.0;
    final rect = Rect.fromLTWH(x, 0, barWidth, size.height);
    canvas.drawRect(rect, Paint()..color = trough);
    canvas.drawRect(
      rect.deflate(0.5),
      Paint()
        ..color = edge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
    // On the fader's own scale, so the numbers printed beside the rail read
    // the meter too: a bar level with the 0 mark is at 0 dB.
    double yOf(double amplitude) {
      // Under the meter's floor is silence, and draws nothing.
      final db = amplitudeDb(amplitude);
      final f = db <= -60 ? 0.0 : _faderFraction(db);
      return _knobHeight / 2 + (size.height - _knobHeight) * (1 - f);
    }

    // Peak behind, dimmer; RMS in front — the bar's body is what it sounds
    // like, the cap is the loudest instant.
    final floor = yOf(0);
    final peakTop = yOf(peak);
    if (peakTop < floor) {
      canvas.drawRect(
        Rect.fromLTRB(x, peakTop, x + barWidth, size.height),
        Paint()..color = cap.withValues(alpha: 0.55),
      );
    }
    final rmsTop = yOf(rms);
    if (rmsTop < floor) {
      canvas.drawRect(
        Rect.fromLTRB(x, rmsTop, x + barWidth, size.height),
        Paint()..color = body,
      );
    }
    if (yOf(holdValue) < floor) {
      final y = yOf(holdValue);
      canvas.drawRect(
        Rect.fromLTWH(x, y, barWidth, 1),
        Paint()..color = hold,
      );
    }
  }

  /// The feed's notifier drives every repaint; a rebuild with the same wiring
  /// has nothing new to draw.
  @override
  bool shouldRepaint(MeterPainter old) =>
      old.stripId != stripId || old.frame != frame || old.body != body;
}

/// The fader: a vertical drag over a rail, with the console's scale printed
/// beside it. The knob moves by as much as the pointer does, wherever it was
/// taken hold of, so a press a hair off the knob never jumps the level. Live
/// values go up to the strip and the release commits once; double-click
/// returns to unity.
class _Fader extends StatefulWidget {
  final double db;
  final ValueChanged<double> onLive;
  final ValueChanged<double> onCommit;
  final VoidCallback onCancel;

  const _Fader({
    super.key,
    required this.db,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
  });

  @override
  State<_Fader> createState() => _FaderState();
}

/// The marks the scale prints, as a console's are: no minus signs, the floor
/// left to the rail's own foot.
const List<(double, String)> _faderScale = [
  (6, '6'),
  (0, '0'),
  (-10, '10'),
  (-20, '20'),
  (-40, '40'),
];

/// How tall the knob is drawn.
const double _knobHeight = 7;

class _FaderState extends State<_Fader> with TickerProviderStateMixin {
  late final HoverGlow _glow = HoverGlow(this, () => setState(() {}));
  double _from = 0;
  double _travel = 0;

  @override
  void dispose() {
    _glow.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final motion = scope.motion;
    return LayoutBuilder(
      builder: (context, constraints) {
        final throw_ = math.max(1.0, constraints.maxHeight - _knobHeight);
        return MouseRegion(
          cursor: SystemMouseCursors.resizeUpDown,
          onEnter: (_) => _glow.mark(motion, over: true),
          onExit: (_) => _glow.mark(motion, over: false),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragStart: (_) {
              _from = _faderFraction(widget.db);
              _travel = 0;
              _glow.mark(motion, held: true);
            },
            onVerticalDragUpdate: (d) {
              _travel += d.delta.dy;
              widget.onLive(_faderDb(_from - _travel / throw_));
            },
            // The strip fed [db] every live tick, so what is showing is what
            // the release commits.
            onVerticalDragEnd: (_) {
              _glow.mark(motion, held: false);
              widget.onCommit(widget.db);
            },
            onVerticalDragCancel: () {
              _glow.mark(motion, held: false);
              widget.onCancel();
            },
            onDoubleTap: () => widget.onCommit(0.0),
            child: SizedBox(
              width: 26,
              child: CustomPaint(
                painter: _FaderPainter(
                  fraction: _faderFraction(widget.db),
                  rail: t.surface3,
                  knob: Color.lerp(t.accent, t.textPrimary, _glow.light * 0.3)!,
                  tick: t.textMuted,
                  label: t.mono.copyWith(fontSize: 9, color: t.textMuted),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

class _FaderPainter extends CustomPainter {
  final double fraction;
  final Color rail, knob, tick;
  final TextStyle label;

  const _FaderPainter({
    required this.fraction,
    required this.rail,
    required this.knob,
    required this.tick,
    required this.label,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // The rail stands at the right of the box, the scale's words to its left.
    final x = size.width - 7;
    final travel = size.height - _knobHeight;
    double yOf(double f) => _knobHeight / 2 + travel * (1 - f);
    canvas.drawRect(
      Rect.fromLTWH(x - 1.5, 0, 3, size.height),
      Paint()..color = rail,
    );
    for (final (db, text) in _faderScale) {
      final y = yOf(_faderFraction(db));
      // Unity is the one a hand looks for, so its mark is the longer.
      final unity = db == 0;
      canvas.drawRect(
        Rect.fromLTWH(x - (unity ? 7 : 5), y - 0.5, unity ? 4 : 2, 1),
        Paint()..color = tick,
      );
      final words = TextPainter(
        text: TextSpan(text: text, style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      words.paint(
          canvas, Offset(x - 9 - words.width, y - words.height / 2));
    }
    final y = yOf(fraction);
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(x - 6.5, y - _knobHeight / 2, 13, _knobHeight),
        const Radius.circular(1),
      ),
      Paint()..color = knob,
    );
    // The knob's own line, which is where the level reads.
    canvas.drawRect(
      Rect.fromLTWH(x - 4.5, y - 0.5, 9, 1),
      Paint()..color = rail,
    );
  }

  @override
  bool shouldRepaint(_FaderPainter old) =>
      old.fraction != fraction ||
      old.knob != knob ||
      old.rail != rail ||
      old.tick != tick;
}

/// The pan pot: a small dial, −100 full left through centre to +100 full
/// right. Vertical drag turns it; double-click recentres. Public
/// because the board draws the same pot on the Audio panel's Pan row, and one
/// dial must turn the same way everywhere.
class PanPot extends StatelessWidget {
  final double value;
  final ValueChanged<double> onLive;
  final ValueChanged<double> onCommit;
  final VoidCallback onCancel;

  const PanPot({
    super.key,
    required this.value,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    var live = value;
    return LumitTooltip(
      message: panLabel(value),
      child: HoverMark(
        cursor: SystemMouseCursors.resizeUpDown,
        builder: (context, lit) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onVerticalDragUpdate: (d) {
          live = (live - d.delta.dy * 2).clamp(-100.0, 100.0);
          onLive(live);
        },
        onVerticalDragEnd: (_) => onCommit(value),
        onVerticalDragCancel: onCancel,
        onDoubleTap: () => onCommit(0.0),
        child: SizedBox(
          width: 14,
          height: 14,
          child: CustomPaint(
            painter: _PotPainter(
              fraction: (value + 100) / 200,
              ring: Color.lerp(t.hairlineStrong, t.textSecondary, lit)!,
              mark: Color.lerp(t.accent, t.textPrimary, lit * 0.3)!,
            ),
          ),
        ),
        ),
      ),
    );
  }
}

/// The pan value as the well reads it: "C", "L 50", "R 30" — a
/// percentage of the way to one side, no arithmetic required.
String panLabel(double pan) {
  final rounded = pan.round();
  if (rounded == 0) return l10n.panCentre;
  return rounded < 0 ? 'L ${-rounded}' : 'R $rounded';
}

class _PotPainter extends CustomPainter {
  final double fraction;
  final Color ring, mark;

  const _PotPainter({
    required this.fraction,
    required this.ring,
    required this.mark,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final centre = size.center(Offset.zero);
    final radius = size.shortestSide / 2 - 0.5;
    canvas.drawCircle(
      centre,
      radius,
      Paint()
        ..color = ring
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
    // Straight up at centre, ±135° at the ends — a pot's throw.
    final angle = (fraction - 0.5) * 1.5 * math.pi;
    final dir =
        Offset(math.sin(angle), -math.cos(angle)) * (radius * 0.9);
    canvas.drawLine(
      centre + dir * 0.25,
      centre + dir,
      Paint()
        ..color = mark
        ..strokeWidth = 1.2,
    );
  }

  @override
  bool shouldRepaint(_PotPainter old) =>
      old.fraction != fraction || old.mark != mark || old.ring != ring;
}

/// The dB well under a fader.
class _DbWell extends StatelessWidget {
  final double db;

  const _DbWell({required this.db});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: 54,
      height: 15,
      alignment: Alignment.centerRight,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      decoration: BoxDecoration(
        color: t.surface0,
        border: Border.all(color: t.hairline),
        borderRadius: BorderRadius.circular(2),
      ),
      child: Text(
        '${db <= _faderFloorDb ? l10n.volumeNegInf : db.toStringAsFixed(1)} dB',
        style: t.mono.copyWith(fontSize: 9, color: t.textPrimary),
        maxLines: 1,
      ),
    );
  }
}

class _SwitchGlyph extends StatelessWidget {
  final String keyName;
  final String mark;
  final bool on;
  final String tip;
  final VoidCallback onTap;

  const _SwitchGlyph({
    required this.keyName,
    required this.mark,
    required this.on,
    required this.tip,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    // A strip that sounds is the usual case and draws quietly; a muted one is
    // the one to notice, so it takes the warning colour.
    return LumitTooltip(
      message: tip,
      child: HoverMark(
        builder: (context, lit) => GestureDetector(
          key: ValueKey(keyName),
          onTap: onTap,
          behavior: HitTestBehavior.opaque,
          child: TweenAnimationBuilder<Color?>(
            tween: ColorTween(end: on ? t.textMuted : t.warning),
            duration: scope.motion.mark.duration,
            curve: scope.motion.mark.curve,
            builder: (context, ink, _) => glyph.LumitIcon(
              mark,
              size: 12,
              colour: Color.lerp(ink, t.textPrimary, on ? lit : 0),
            ),
          ),
        ),
      ),
    );
  }
}

class _SoloChip extends StatelessWidget {
  final String keyName;
  final bool on;
  final VoidCallback onTap;

  const _SoloChip({required this.keyName, required this.on, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    return LumitTooltip(
      message: on ? l10n.switchSoloed : l10n.switchSolo,
      child: HoverMark(
        builder: (context, lit) => GestureDetector(
          key: ValueKey(keyName),
          onTap: onTap,
          behavior: HitTestBehavior.opaque,
          child: AnimatedContainer(
            duration: scope.motion.mark.duration,
            curve: scope.motion.mark.curve,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            decoration: BoxDecoration(
              color: on
                  ? t.accent.withValues(alpha: 0.25)
                  : t.accent.withValues(alpha: 0),
              border: Border.all(color: on ? t.accent : t.hairlineStrong),
              borderRadius: BorderRadius.circular(2),
            ),
            child: Text(
              'S',
              style: t.mono.copyWith(
                fontSize: 9,
                color: on
                    ? t.textPrimary
                    : Color.lerp(t.textMuted, t.textSecondary, lit),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
