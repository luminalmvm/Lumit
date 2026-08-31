// The Mixer panel (docs/09 §3.1, K-690/K-691, the approved AudioWorkspace
// board): one strip per audible row of the fronted comp, in comp order, and
// the Master strip at the right-hand edge.
//
// Each strip is the board's drawing: the layer's name underlined in its label
// colour, a pan pot, a fader beside the stereo meters, the dB well, and the
// mute and solo switches. The Master strip swaps the pot for the limiter lamp
// and carries the muted LUFS placeholder (loudness measurement is post-v1,
// docs/09 §8).
//
// **The meters animate; nothing else does.** One [AudioMeterFeed] polls the
// engine's tap and the meter *painters* listen to its frame directly, each
// inside its own RepaintBoundary — a tick repaints the bars and rebuilds no
// widget at all, which is what keeps the K-681 redraw gates green
// (docs/impl/ui-performance.md WP-2). Everything else draws from the held comp
// model and rebuilds only when the document moves.

import 'dart:math' as math;

import 'package:flutter/foundation.dart' show ValueListenable;
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
import '../state/audio_effects.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'audio_meters_feed.dart';
import 'keyframe_controls_frb.dart';
import 'placeholder.dart';

/// The fader's travel in decibels: silence to a small push. The +12 the value
/// field allows is still reachable by typing in the Audio panel; a fader that
/// spent a third of its throw above unity would waste it.
const double _faderFloorDb = -60.0;
const double _faderCeilingDb = 6.0;

double _faderFraction(double db) =>
    ((db - _faderFloorDb) / (_faderCeilingDb - _faderFloorDb)).clamp(0.0, 1.0);

double _faderDb(double fraction) =>
    _faderFloorDb + fraction.clamp(0.0, 1.0) * (_faderCeilingDb - _faderFloorDb);

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

  /// Which layers can make a sound, probed once per layer and remembered —
  /// the K-435 question, asked exactly as the Timeline asks it.
  final Map<String, bool> _hasAudio = {};

  @override
  void initState() {
    super.initState();
    if (widget.feed == null) _feed.start();
  }

  @override
  void dispose() {
    if (widget.feed == null) _feed.dispose();
    super.dispose();
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
                    : ListView(
                        scrollDirection: Axis.horizontal,
                        children: [
                          for (final entry in sounding)
                            _MixerStrip(
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
                        ],
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
    // rebuilds for one (the K-681 discipline).
    final animated = info.volumeDb is! BridgeScalar_Static ||
        info.pan is! BridgeScalar_Static;

    return Container(
      width: _stripWidth,
      decoration: BoxDecoration(
        border: Border(right: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: !animated
          ? _body(context, t, info, layer, id, switches, playhead.value)
          : ValueListenableBuilder<int>(
              valueListenable: playhead,
              builder: (context, frame, _) =>
                  _body(context, t, info, layer, id, switches, frame),
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
                      .where((e) => e.enabled && isAudioEffectName(e.name))
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
              _PanPot(
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
                    const SizedBox(width: 5),
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

/// The Master strip (K-691): the fader is a gain stage on the sum, the lamp is
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
                const SizedBox(width: 5),
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
            style: t.small.copyWith(fontSize: 8, color: t.textDisabled),
          ),
        ],
      ),
    );
  }
}

/// The limiter's lamp (K-535/K-691): LIM, lit while the sticky clip flag is
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
        child: GestureDetector(
          key: const ValueKey('limiter-lamp'),
          onTap: resetAudioClip,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
            decoration: BoxDecoration(
              border: Border.all(color: lit ? t.error : t.hairlineStrong),
              borderRadius: BorderRadius.circular(2),
            ),
            child: Text(
              'LIM',
              style: t.mono.copyWith(
                fontSize: 9,
                color: lit ? t.error : t.textMuted,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// One strip's stereo meters: RMS body, peak cap, and the panel's own hold
/// line, on a −60..0 dB scale.
///
/// A [RepaintBoundary] around a [CustomPaint] whose painter repaints off the
/// feed's frame notifier directly — a meter tick is a repaint of this band
/// and a rebuild of nothing (the K-681 shape).
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
    // Peak behind, dimmer; RMS in front — the bar's body is what it sounds
    // like, the cap is the loudest instant.
    final peakTop = size.height * (1 - meterFraction(peak));
    canvas.drawRect(
      Rect.fromLTRB(x, peakTop, x + barWidth, size.height),
      Paint()..color = cap.withValues(alpha: 0.55),
    );
    final rmsTop = size.height * (1 - meterFraction(rms));
    canvas.drawRect(
      Rect.fromLTRB(x, rmsTop, x + barWidth, size.height),
      Paint()..color = body,
    );
    if (holdValue > 0) {
      final y = size.height * (1 - meterFraction(holdValue));
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

/// The fader: a vertical drag over a rail. Live values go up to the strip and
/// the release commits once; double-click returns to unity.
class _Fader extends StatelessWidget {
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
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, constraints) {
        final height = constraints.maxHeight;
        double dbAt(Offset local) =>
            _faderDb(1 - (local.dy / height).clamp(0.0, 1.0));
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onVerticalDragUpdate: (d) => onLive(dbAt(d.localPosition)),
          // The strip fed [db] every live tick, so what is showing is what
          // the release commits.
          onVerticalDragEnd: (_) => onCommit(db),
          onVerticalDragCancel: onCancel,
          onDoubleTap: () => onCommit(0.0),
          child: SizedBox(
            width: 13,
            child: CustomPaint(
              painter: _FaderPainter(
                fraction: _faderFraction(db),
                rail: t.surface3,
                knob: t.textSecondary,
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
  final Color rail, knob;

  const _FaderPainter({
    required this.fraction,
    required this.rail,
    required this.knob,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final x = size.width / 2;
    canvas.drawRect(
      Rect.fromLTWH(x - 1.5, 0, 3, size.height),
      Paint()..color = rail,
    );
    final y = (size.height - 5) * (1 - fraction);
    canvas.drawRect(
      Rect.fromLTWH(x - 5.5, y, 11, 5),
      Paint()..color = knob,
    );
  }

  @override
  bool shouldRepaint(_FaderPainter old) =>
      old.fraction != fraction || old.knob != knob;
}

/// The pan pot: a small dial, −100 full left through centre to +100 full
/// right (K-694). Vertical drag turns it; double-click recentres.
class _PanPot extends StatelessWidget {
  final double value;
  final ValueChanged<double> onLive;
  final ValueChanged<double> onCommit;
  final VoidCallback onCancel;

  const _PanPot({
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
      child: GestureDetector(
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
              ring: t.hairlineStrong,
              mark: t.textSecondary,
            ),
          ),
        ),
      ),
    );
  }
}

/// The pan value as the well reads it (K-694): "C", "L 50", "R 30" — a
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
      old.fraction != fraction || old.mark != mark;
}

/// The dB well under a fader.
class _DbWell extends StatelessWidget {
  final double db;

  const _DbWell({required this.db});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: 44,
      height: 15,
      alignment: Alignment.centerRight,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      decoration: BoxDecoration(
        color: t.surface0,
        border: Border.all(color: t.hairline),
        borderRadius: BorderRadius.circular(2),
      ),
      child: Text(
        db <= -60 ? l10n.volumeNegInf : db.toStringAsFixed(1),
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
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: tip,
      child: GestureDetector(
        key: ValueKey(keyName),
        onTap: onTap,
        behavior: HitTestBehavior.opaque,
        child: glyph.LumitIcon(
          mark,
          size: 12,
          colour: on ? t.warning : t.textMuted,
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
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: on ? l10n.switchSoloed : l10n.switchSolo,
      child: GestureDetector(
        key: ValueKey(keyName),
        onTap: onTap,
        behavior: HitTestBehavior.opaque,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 4),
          decoration: BoxDecoration(
            border: Border.all(color: on ? t.accent : t.hairlineStrong),
            borderRadius: BorderRadius.circular(2),
          ),
          child: Text(
            'S',
            style: t.mono.copyWith(
              fontSize: 9,
              color: on ? t.textPrimary : t.textMuted,
            ),
          ),
        ),
      ),
    );
  }
}
