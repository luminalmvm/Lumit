// The Audio panel (docs/09, the approved AudioWorkspace board): three
// sections down a column.
//
// **Levels** — the output's stereo bars off the engine's tap (K-690), with the
// sticky clip lamp and the peak-hold caption. **Beats** — the full face of the
// beat engine (docs/09 §5): source, sensitivity, range, minimum spacing, the
// BPM well with Tap and the phase chips, and the Generate/Clear pair.
// **Selected layer** — the fronted selection's sound: Volume and Pan rows with
// their stopwatches, the fade wells with their curve chips (K-695), and the
// two graph-template buttons, whose staged chains are ordinary K-471 wires the
// Graph panel draws and the user can retune or delete.
//
// The bars animate at UI rate inside their own RepaintBoundary off the shared
// meter feed; everything else rebuilds only when the document or the
// selection moves (the K-681 gates).

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/beats.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/splash.dart';
import '../state/app_state.dart';
import '../state/beats_notice.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'audio_meters_feed.dart';
import 'keyframe_controls_frb.dart';
import 'mixer_panel_frb.dart' show PanPot, panLabel;
import 'placeholder.dart';

/// What the Range dropdown offers (docs/09 §5): the whole comp, or only the
/// work area.
enum BeatRange { wholeComp, workArea }

class AudioPanelFrb extends StatefulWidget {
  /// A test's own feed, silent and hand-pulsed; the application passes none.
  final AudioMeterFeed? feed;

  const AudioPanelFrb({super.key, this.feed});

  @override
  State<AudioPanelFrb> createState() => _AudioPanelFrbState();
}

class _AudioPanelFrbState extends State<AudioPanelFrb> {
  late final AudioMeterFeed _feed = widget.feed ?? AudioMeterFeed();

  // --- The Beats section's own settings, panel state until Generate runs.
  /// Empty string is the comp's own mix, else a layer id — the same two
  /// shapes the engine's Source takes.
  String _source = '';
  double _sensitivity = 50;
  BeatRange _range = BeatRange.wholeComp;
  int _minSpacingMs = 120;

  /// The BPM well: zero follows the estimate, anything else is the override
  /// the grid snaps to. Tap writes it; the last Generate's answer fills the
  /// well when no override stands.
  double _bpmOverride = 0;

  /// What the last detection reported, for the well to read back.
  double _lastBpm = 0;
  double _phaseMs = 0;

  /// Tap tempo: the last few tap moments; the median gap is the tempo.
  final List<DateTime> _taps = [];

  /// Which layers can make a sound, probed once and remembered (K-435).
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
        title: l10n.panelAudio,
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
          child: ListView(
            children: [
              _section(t, l10n.audioSectionLevels),
              _LevelsBlock(feed: _feed),
              _hair(t),
              _section(t, l10n.audioSectionBeats),
              _beats(context, t, ui, comp, sounding),
              _hair(t),
              _selectedLayer(context, t, ui, comp),
            ],
          ),
        );
      },
    );
  }

  Widget _section(LumitTheme t, String label) => Padding(
        padding: const EdgeInsets.fromLTRB(10, 8, 10, 4),
        child: Text(label, style: t.kicker),
      );

  Widget _hair(LumitTheme t) => Container(height: 1, color: t.hairline);

  Widget _label(LumitTheme t, String text) => SizedBox(
        width: 74,
        child: Text(text, style: t.body.copyWith(color: t.textMuted)),
      );

  // --- Beats -------------------------------------------------------------

  Widget _beats(
    BuildContext context,
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp,
    List<BridgeLayerEntry> sounding,
  ) {
    final bpmShown = _bpmOverride > 0 ? _bpmOverride : _lastBpm;
    String sourceName(String id) {
      if (id.isEmpty) return l10n.fxAudioThisComp;
      for (final entry in sounding) {
        if (entry.layer.internallayerId.toString() == id) {
          return entry.info.name;
        }
      }
      return l10n.fxAudioThisComp;
    }

    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 0, 10, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(children: [
            _label(t, l10n.beatsSource),
            Expanded(
              child: BareDropdown<String>(
                key: const ValueKey('beats-source'),
                value: _source,
                options: [
                  '',
                  for (final entry in sounding)
                    entry.layer.internallayerId.toString(),
                ],
                label: sourceName,
                onChanged: (v) => setState(() => _source = v),
              ),
            ),
          ]),
          const SizedBox(height: 6),
          Row(children: [
            _label(t, l10n.beatsSensitivity),
            Expanded(
              child: HouseSlider(
                key: const ValueKey('beats-sensitivity'),
                value: _sensitivity,
                min: 0,
                max: 100,
                decimals: 0,
                // Fits beside its value readout at the panel's floor width;
                // the slider does not stretch (its rail is its travel).
                width: 110,
                onChanged: (v) => setState(() => _sensitivity = v),
              ),
            ),
          ]),
          const SizedBox(height: 6),
          Row(children: [
            _label(t, l10n.beatsRange),
            Expanded(
              child: BareDropdown<BeatRange>(
                key: const ValueKey('beats-range'),
                value: _range,
                options: BeatRange.values,
                label: (r) => switch (r) {
                  BeatRange.wholeComp => l10n.beatsRangeWholeComp,
                  BeatRange.workArea => l10n.beatsRangeWorkArea,
                },
                onChanged: (v) => setState(() => _range = v),
              ),
            ),
          ]),
          const SizedBox(height: 6),
          Row(children: [
            _label(t, l10n.beatsMinSpacing),
            SizedBox(
              width: 64,
              child: DragValueField(
                key: const ValueKey('beats-spacing'),
                value: _minSpacingMs.toDouble(),
                min: 0,
                max: 1000,
                decimals: 0,
                suffix: ' ms',
                onChanged: (v) =>
                    setState(() => _minSpacingMs = v.round().clamp(0, 1000)),
              ),
            ),
          ]),
          const SizedBox(height: 6),
          // Wraps rather than rows from here down: a docked-narrow panel
          // folds a line rather than painting outside its box (K-451).
          Wrap(
            spacing: 6,
            runSpacing: 4,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              _label(t, l10n.beatsBpm),
              SizedBox(
                width: 64,
                child: DragValueField(
                  key: const ValueKey('beats-bpm'),
                  value: bpmShown,
                  min: 0,
                  max: 300,
                  decimals: 2,
                  onChanged: (v) =>
                      setState(() => _bpmOverride = v.toDouble().clamp(0, 300)),
                ),
              ),
              HouseButton(
                key: const ValueKey('beats-tap'),
                small: true,
                onPressed: _tap,
                child: Text(l10n.beatsTap),
              ),
              // The phase chips: the grid is right but early or late. Ten
              // milliseconds a press — under half a frame at 60.
              _phaseChip(t, '‹', -10),
              _phaseChip(t, '›', 10),
              if (_bpmOverride > 0) Text(l10n.beatsGridOn, style: t.caption),
            ],
          ),
          const SizedBox(height: 8),
          Wrap(
            spacing: 8,
            runSpacing: 4,
            children: [
              HouseButton(
                key: const ValueKey('beats-generate'),
                small: true,
                onPressed: () => _generate(context, ui, comp),
                child: Text(l10n.beatsGenerate),
              ),
              HouseButton(
                key: const ValueKey('beats-clear'),
                small: true,
                onPressed: () {
                  comp.clearBeatMarkers();
                  ui.model.refresh();
                },
                child: Text(l10n.beatsClearGenerated),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _phaseChip(LumitTheme t, String glyphText, double deltaMs) =>
      GestureDetector(
        key: ValueKey('beats-phase-${deltaMs > 0 ? 'later' : 'earlier'}'),
        onTap: () => setState(() => _phaseMs += deltaMs),
        child: LumitTooltip(
          message:
              deltaMs > 0 ? l10n.beatsPhaseLater : l10n.beatsPhaseEarlier,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 5),
            decoration: BoxDecoration(
              border: Border.all(color: t.hairlineStrong),
              borderRadius: BorderRadius.circular(2),
            ),
            child: Text(glyphText,
                style: t.mono.copyWith(fontSize: 10, color: t.textSecondary)),
          ),
        ),
      );

  /// Tap tempo (docs/09 §5): the median gap of the last taps becomes the BPM
  /// well's override. A pause of two seconds starts a fresh count.
  void _tap() {
    final now = DateTime.now();
    if (_taps.isNotEmpty &&
        now.difference(_taps.last) > const Duration(seconds: 2)) {
      _taps.clear();
    }
    _taps.add(now);
    while (_taps.length > 9) {
      _taps.removeAt(0);
    }
    if (_taps.length < 2) return;
    final gaps = <int>[
      for (var i = 1; i < _taps.length; i++)
        _taps[i].difference(_taps[i - 1]).inMilliseconds,
    ]..sort();
    final median = gaps[gaps.length ~/ 2];
    if (median <= 0) return;
    setState(() => _bpmOverride =
        double.parse((60000.0 / median).clamp(30, 300).toStringAsFixed(2)));
  }

  void _generate(
      BuildContext context, LumitUiState ui, CompositionReference comp) {
    final options = BridgeBeatOptions(
      sourceLayer: _source,
      sensitivityPercent: _sensitivity.round(),
      workAreaOnly: _range == BeatRange.workArea,
      minSpacingMs: _minSpacingMs,
      bpmOverride: _bpmOverride,
      phaseMs: _phaseMs,
    );
    // The same busy card the toolbar's one-click detection shows: detection
    // reads whole files and can take seconds, and silence would read as a
    // command that did not land.
    //
    // **A refusal says why.** This used to be `onError: (_) {}`, so a comp
    // whose mix is silenced — a soloed picture row (K-435) is the everyday way
    // — placed no markers, cleared the grid and explained nothing. It is one
    // sentence per *source* rather than one per reason because a `BridgeError`
    // reaches Dart as an opaque handle with nothing readable on it: `NoAudio`
    // is what the engine answers here in every case a person can cause
    // (docs/09 §5), the rest being a project that closed.
    //
    // Which sentence is the source's, not the error's, and the panel is the
    // one that knows it: a mute or a solo cannot silence a layer picked by
    // name (K-718), so blaming them would send the reader to the wrong switch.
    // ponytail: two sentences for the whole refusal; split further the day
    // BridgeError carries a reason id across the bridge.
    final app = context.read<LumitState>();
    showBusyWhile(
      app.busy,
      l10n.detectingBeats,
      comp.detectBeats(options: options).then<void>((found) {
        if (mounted) setState(() => _lastBpm = found.bpm);
        ui.model.refresh();
        // A run that placed nothing is a legitimate answer (docs/09 §5) and
        // used to be an indistinguishable one: no markers, no grid, no word.
        // A run that placed markers says so too — the board's own status
        // caption — because the markers land off-screen as easily as on.
        app.postNotice(
            found.placed == 0 ? l10n.beatsNoneFound : beatsFoundNotice(found));
      },
          onError: (_) => app.postNotice(
              _source.isEmpty ? l10n.beatsNoSound : l10n.beatsLayerNoSound)),
    );
  }

  // --- Selected layer -----------------------------------------------------

  Widget _selectedLayer(
    BuildContext context,
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp,
  ) {
    final selected = ui.selectedLayerIds;
    BridgeLayerEntry? entry;
    for (final candidate in ui.model.heldLayers) {
      if (selected.contains(candidate.layer.internallayerId)) {
        entry = candidate;
        break;
      }
    }
    final canSound = entry != null &&
        (_hasAudio[entry.layer.internallayerId.toString()] ?? false);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(10, 8, 10, 4),
          child: Row(children: [
            Text(l10n.audioSectionSelectedLayer, style: t.kicker),
            const SizedBox(width: 8),
            if (entry != null)
              Expanded(
                child: Text(
                  entry.info.name,
                  style: t.mono.copyWith(fontSize: 10, color: t.textPrimary),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
          ]),
        ),
        if (entry == null)
          Padding(
            padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
            child: Text(l10n.selectALayer, style: t.small),
          )
        else
          _SelectedLayerBlock(
            key: ValueKey('audio-selected-${entry.layer.internallayerId}'),
            comp: comp,
            entry: entry,
            // A silent layer keeps the template buttons — *Drive with audio…*
            // exists precisely to move a visual layer's parameters with the
            // music — and loses the rows that would be lying on it.
            canSound: canSound,
            onChanged: () {
              ui.model.refresh();
              comp.audioPrepare();
            },
          ),
      ],
    );
  }
}

/// The output's two bars, dB readouts, and the clip row — the board's Levels
/// block. The bars and their numbers repaint off the feed inside one
/// RepaintBoundary; the clip lamp is its own tiny widget under the clip
/// listenable.
class _LevelsBlock extends StatelessWidget {
  final AudioMeterFeed feed;

  const _LevelsBlock({required this.feed});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 0, 10, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          RepaintBoundary(
            key: const ValueKey('levels-bars'),
            child: SizedBox(
              height: 26,
              child: CustomPaint(
                painter: LevelBarsPainter(
                  frame: feed.frame,
                  trough: t.surface0,
                  edge: t.hairline,
                  body: t.success,
                  cap: t.warning,
                  hold: t.textPrimary,
                  text: t.mono.copyWith(fontSize: 9, color: t.textMuted),
                ),
              ),
            ),
          ),
          const SizedBox(height: 4),
          Row(children: [
            Text(l10n.audioClip, style: t.caption),
            const SizedBox(width: 6),
            ValueListenableBuilder<bool>(
              valueListenable: feed.clipped,
              builder: (context, lit, _) => LumitTooltip(
                message:
                    lit ? l10n.mixerLimiterClipped : l10n.mixerLimiter,
                child: GestureDetector(
                  key: const ValueKey('audio-clip-lamp'),
                  onTap: resetAudioClip,
                  child: Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(
                      color: lit ? t.error : t.surface2,
                      border: Border.all(color: t.hairlineStrong),
                      borderRadius: BorderRadius.circular(1),
                    ),
                  ),
                ),
              ),
            ),
            const Spacer(),
            Text(l10n.audioPeakHold, style: t.caption),
          ]),
        ],
      ),
    );
  }
}

/// The two horizontal bars with their dB numbers, painted as one so a tick
/// repaints one boundary and rebuilds nothing.
class LevelBarsPainter extends CustomPainter {
  final ValueListenable<AudioMeterFrame> frame;
  final Color trough, edge, body, cap, hold;
  final TextStyle text;

  LevelBarsPainter({
    required this.frame,
    required this.trough,
    required this.edge,
    required this.body,
    required this.cap,
    required this.hold,
    required this.text,
  }) : super(repaint: frame);

  @override
  void paint(Canvas canvas, Size size) {
    final master = frame.value.master;
    _bar(canvas, size, 0, master.rmsLeft, master.peakLeft, master.holdLeft);
    _bar(canvas, size, 1, master.rmsRight, master.peakRight, master.holdRight);
  }

  void _bar(Canvas canvas, Size size, int row, double rms, double peak,
      double holdValue) {
    const textWidth = 44.0;
    const barHeight = 5.0;
    final width = size.width - textWidth;
    final y = row * 14.0 + (5 - barHeight) / 2;
    final rect = Rect.fromLTWH(0, y, width, barHeight);
    canvas.drawRect(rect, Paint()..color = trough);
    canvas.drawRect(
      rect.deflate(0.5),
      Paint()
        ..color = edge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
    canvas.drawRect(
      Rect.fromLTWH(0, y, width * meterFraction(peak), barHeight),
      Paint()..color = cap.withValues(alpha: 0.55),
    );
    canvas.drawRect(
      Rect.fromLTWH(0, y, width * meterFraction(rms), barHeight),
      Paint()..color = body,
    );
    if (holdValue > 0) {
      canvas.drawRect(
        Rect.fromLTWH(width * meterFraction(holdValue) - 1, y, 1, barHeight),
        Paint()..color = hold,
      );
    }
    final db = amplitudeDb(peak);
    final painter = TextPainter(
      text: TextSpan(
        text: db <= -60 ? l10n.volumeNegInf : '${db.toStringAsFixed(1)} dB',
        style: text,
      ),
      textDirection: TextDirection.ltr,
      textAlign: TextAlign.right,
    )..layout(minWidth: textWidth - 4);
    painter.paint(canvas, Offset(width + 4, y - 3));
  }

  @override
  bool shouldRepaint(LevelBarsPainter old) =>
      old.frame != frame || old.body != body || old.text != text;
}

/// The selection's sound: Volume and Pan with their stopwatches, the fades,
/// and the two template buttons.
class _SelectedLayerBlock extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

  /// Whether the layer can make a sound (K-435). Off, the Volume, Pan and
  /// fade rows are not drawn — controls on a silent layer would be switches
  /// that switch nothing — and the template buttons stay.
  final bool canSound;
  final VoidCallback onChanged;

  const _SelectedLayerBlock({
    super.key,
    required this.comp,
    required this.entry,
    required this.canSound,
    required this.onChanged,
  });

  @override
  State<_SelectedLayerBlock> createState() => _SelectedLayerBlockState();
}

class _SelectedLayerBlockState extends State<_SelectedLayerBlock> {
  /// The fade wells' seconds — panel state until a commit writes the keys.
  double _fadeInSeconds = 0.5;
  double _fadeOutSeconds = 0.5;
  BridgeFadeShape _fadeInShape = BridgeFadeShape.ease;
  BridgeFadeShape _fadeOutShape = BridgeFadeShape.ease;

  /// The pot's live reading while it is being turned, or null at rest — the
  /// Mixer strip's own shape: the drag shows here and commits once on release.
  double? _dragPan;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final info = widget.entry.info;
    final layer = widget.entry.layer;

    return ValueListenableBuilder<int>(
      valueListenable: ui.playheadFrame,
      builder: (context, frame, _) {
        final volume = info.volumeDb;
        final pan = info.pan;
        final volumeValue = volume is BridgeScalar_Static
            ? volume.field0
            : sampledScalar(volume, timeOfFrame(widget.comp, frame));
        final panValue = _dragPan ??
            (pan is BridgeScalar_Static
                ? pan.field0
                : sampledScalar(pan, timeOfFrame(widget.comp, frame)));
        return Padding(
          padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!widget.canSound)
                Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Text(l10n.audioLayerSilent, style: t.small),
                ),
              if (widget.canSound) ...[
              Wrap(
                  runSpacing: 4,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                KeyframeControlsFrb(
                  scalars: [volume],
                  comp: widget.comp,
                  playheadFrame: frame,
                  onSeek: ui.scrubTo,
                  rowKey: 'audio-volume',
                  onWrite: (next) {
                    layer.setVolumeDb(value: next.single);
                    widget.onChanged();
                  },
                ),
                SizedBox(
                  width: 56,
                  child: Text(l10n.volume,
                      style: t.body.copyWith(color: t.textMuted)),
                ),
                SizedBox(
                  width: 72,
                  child: DragValueField(
                    key: const ValueKey('audio-volume-db'),
                    value: volumeValue,
                    min: -60,
                    max: 12,
                    decimals: 1,
                    suffix: ' dB',
                    speed: 0.2,
                    onChanged: (v) =>
                        _commitVolume(volume, v.toDouble(), frame),
                  ),
                ),
              ]),
              const SizedBox(height: 6),
              Wrap(
                  runSpacing: 4,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                KeyframeControlsFrb(
                  scalars: [pan],
                  comp: widget.comp,
                  playheadFrame: frame,
                  onSeek: ui.scrubTo,
                  rowKey: 'audio-pan',
                  onWrite: (next) {
                    layer.setPan(value: next.single);
                    widget.onChanged();
                  },
                ),
                SizedBox(
                  width: 56,
                  child: Text(l10n.audioPan,
                      style: t.body.copyWith(color: t.textMuted)),
                ),
                // The board's own dial beside the value well — the Mixer
                // strip's pot, turning the same way (K-694).
                PanPot(
                  key: const ValueKey('audio-pan-pot'),
                  value: panValue,
                  onLive: (v) => setState(() => _dragPan = v),
                  onCommit: (v) => _commitPan(pan, v, frame),
                  onCancel: () => setState(() => _dragPan = null),
                ),
                const SizedBox(width: 6),
                SizedBox(
                  width: 72,
                  child: DragValueField(
                    key: const ValueKey('audio-pan-value'),
                    value: panValue,
                    min: -100,
                    max: 100,
                    decimals: 0,
                    speed: 0.5,
                    onChanged: (v) => _commitPan(pan, v.toDouble(), frame),
                  ),
                ),
                const SizedBox(width: 6),
                Text(panLabel(panValue), style: t.caption),
              ]),
              const SizedBox(height: 6),
              _fadeRow(
                t,
                key: 'fade-in',
                label: l10n.audioFadeIn,
                seconds: _fadeInSeconds,
                shape: _fadeInShape,
                onSeconds: (v) => setState(() => _fadeInSeconds = v),
                onShape: (s) => setState(() => _fadeInShape = s),
                apply: () {
                  layer.fadeIn(seconds: _fadeInSeconds, shape: _fadeInShape);
                  widget.onChanged();
                },
              ),
              const SizedBox(height: 6),
              _fadeRow(
                t,
                key: 'fade-out',
                label: l10n.audioFadeOut,
                seconds: _fadeOutSeconds,
                shape: _fadeOutShape,
                onSeconds: (v) => setState(() => _fadeOutSeconds = v),
                onShape: (s) => setState(() => _fadeOutShape = s),
                apply: () {
                  layer.fadeOut(seconds: _fadeOutSeconds, shape: _fadeOutShape);
                  widget.onChanged();
                },
              ),
              const SizedBox(height: 10),
              ],
              Wrap(spacing: 8, runSpacing: 4, children: [
                HouseButton(
                  key: const ValueKey('audio-drive'),
                  small: true,
                  onPressed: () => _pickDriveTarget(context),
                  child: Text(l10n.audioDriveWith),
                ),
                HouseButton(
                  key: const ValueKey('audio-duck'),
                  small: true,
                  onPressed: () => _pickDuckSource(context),
                  child: Text(l10n.audioDuckUnder),
                ),
              ]),
            ],
          ),
        );
      },
    );
  }

  /// A fade well and its three curve chips (K-695): the well holds the
  /// length, a chip picks the shape, and either commits the keyframe pair
  /// there and then — running it again reshapes the same fade.
  Widget _fadeRow(
    LumitTheme t, {
    required String key,
    required String label,
    required double seconds,
    required BridgeFadeShape shape,
    required ValueChanged<double> onSeconds,
    required ValueChanged<BridgeFadeShape> onShape,
    required VoidCallback apply,
  }) {
    Widget chip(String text, BridgeFadeShape option) => GestureDetector(
          key: ValueKey('$key-${option.name}'),
          onTap: () {
            onShape(option);
            apply();
          },
          child: Container(
            margin: const EdgeInsets.only(left: 4),
            padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
            decoration: BoxDecoration(
              border: Border.all(
                  color: shape == option ? t.accent : t.hairlineStrong),
              borderRadius: BorderRadius.circular(2),
            ),
            child: Text(
              text,
              style: t.mono.copyWith(
                fontSize: 9,
                color: shape == option ? t.textPrimary : t.textMuted,
              ),
            ),
          ),
        );
    return Wrap(
      spacing: 0,
      runSpacing: 4,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        SizedBox(
          width: 74,
          child: Text(label, style: t.body.copyWith(color: t.textMuted)),
        ),
        SizedBox(
          width: 64,
          child: DragValueField(
            key: ValueKey('$key-seconds'),
            value: seconds,
            min: 0,
            max: 60,
            decimals: 2,
            suffix: ' s',
            speed: 0.02,
            onChanged: (v) {
              onSeconds(v.toDouble());
              apply();
            },
          ),
        ),
        chip(l10n.fadeShapeEase, BridgeFadeShape.ease),
        chip(l10n.fadeShapeLinear, BridgeFadeShape.linear),
        chip(l10n.fadeShapeExponential, BridgeFadeShape.exponential),
      ],
    );
  }

  void _commitVolume(BridgeScalar scalar, double value, int frame) {
    widget.entry.layer.setVolumeDb(
      value: scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    widget.onChanged();
  }

  void _commitPan(BridgeScalar scalar, double value, int frame) {
    widget.entry.layer.setPan(
      value: scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    setState(() => _dragPan = null);
    widget.onChanged();
  }

  // --- The two graph templates (K-471 roads, K-697) -----------------------

  /// *Drive with audio…*: pick an unwired Number parameter of this layer's
  /// stack, then stage Audio level → Remap → Smooth wired onto it — visible
  /// boxes on the Graph panel, one commit, one undo step.
  void _pickDriveTarget(BuildContext context) {
    final layer = widget.entry.layer;
    final BridgeLayerGraph graph;
    try {
      graph = layer.getGraph();
    } catch (_) {
      _sayChainRefused();
      return;
    }
    final targets = <(String, BridgeInputRef)>[];
    for (final node in graph.nodes) {
      if (node.node is! BridgeNodeRef_Effect) continue;
      for (final port in node.inputs) {
        if (port.portType != BridgePortType.number || port.wired) continue;
        targets.add((
          '${engineLabel(node.customName ?? node.label)} › ${engineLabel(port.label)}',
          BridgeInputRef.param(node: node.node, port: port.id),
        ));
      }
    }
    _menu(
      context,
      empty: l10n.audioDriveNoTargets,
      rows: [
        for (final (label, target) in targets)
          (label, () => _stageChain(target: target, inverted: false)),
      ],
    );
  }

  /// *Lower behind…* (the ducking template, in plain words — K-730): pick the
  /// layer whose sound pushes this one down, then
  /// stage the inverted chain onto this layer's own Volume socket (K-697) —
  /// Audio level listening to the picked layer, Remap upside down, Smooth,
  /// into the Layer out's Volume.
  void _pickDuckSource(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final self = widget.entry.layer.internallayerId;
    final others = <(String, UuidValue)>[];
    for (final entry in ui.model.heldLayers) {
      final id = entry.layer.internallayerId;
      if (id == self) continue;
      others.add((entry.info.name, id));
    }
    _menu(
      context,
      empty: l10n.audioDuckNoSources,
      rows: [
        for (final (name, id) in others)
          (
            name,
            () => _stageChain(
                  target: const BridgeInputRef.param(
                    node: BridgeNodeRef.out(),
                    port: 'volume',
                  ),
                  inverted: true,
                  listenTo: id,
                )
          ),
      ],
    );
  }

  void _menu(
    BuildContext context, {
    required String empty,
    required List<(String, VoidCallback)> rows,
  }) {
    final t = ThemeScope.of(context).theme;
    final box = context.findRenderObject() as RenderBox?;
    final at = box == null
        ? Offset.zero
        : box.localToGlobal(box.size.bottomLeft(Offset.zero));
    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (rows.isEmpty)
                Padding(
                  padding: const EdgeInsets.all(8),
                  child: Text(empty, style: t.small),
                ),
              for (final (label, act) in rows)
                MenuRow(
                  onPressed: () {
                    close(null);
                    act();
                  },
                  child: Padding(
                    padding:
                        const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                    child: Text(label, style: t.body),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }

  /// Stage the template: three drivers, three wires, one `setGraph` — so the
  /// whole gesture is one undo step and the Graph panel shows exactly what
  /// was built. An occupied target socket is re-routed rather than doubled,
  /// the same rule a hand-drawn wire follows.
  void _stageChain({
    required BridgeInputRef target,
    required bool inverted,
    UuidValue? listenTo,
  }) {
    final layer = widget.entry.layer;
    final BridgeLayerGraph graph;
    final BridgeEffectInstance level, remap, smooth;
    try {
      graph = layer.getGraph();
      level = layer.newDriver(name: 'audio_level');
      remap = layer.newDriver(name: 'remap');
      smooth = layer.newDriver(name: 'smooth');
    } catch (_) {
      _sayChainRefused();
      return;
    }
    if (listenTo != null) {
      level.setValue(id: 'audio', value: BridgeEffectValue.layer(listenTo));
    }
    // A practical loudness window: RMS of finished music rarely clears 0.3,
    // so the chain reaches its far end on real material rather than only on
    // full scale.
    remap.setValue(
        id: 'in_high',
        value: const BridgeEffectValue.float(BridgeScalar.static_(0.3)));
    if (inverted) {
      // The duck: silence in → 0 dB out, loud in → the floor. −18 dB is a
      // clearly-under bed that is still audibly there; the Remap is an
      // ordinary box, retunable on the graph.
      remap.setValue(
          id: 'out_low',
          value: const BridgeEffectValue.float(BridgeScalar.static_(0.0)));
      remap.setValue(
          id: 'out_high',
          value: const BridgeEffectValue.float(BridgeScalar.static_(-18.0)));
    }
    BridgeGraphEdge wire(
            BridgeEffectInstance from, String port, BridgeInputRef to) =>
        BridgeGraphEdge(
          from: BridgeOutputRef.driver(node: from.id(), port: port),
          to: to,
        );
    final edges = [...graph.wiring.edges]
      ..removeWhere((e) => e.to == target)
      ..addAll([
        wire(level, 'amplitude',
            BridgeInputRef.param(node: BridgeNodeRef.driver(remap.id()), port: 'value')),
        wire(remap, 'value',
            BridgeInputRef.param(node: BridgeNodeRef.driver(smooth.id()), port: 'value')),
        wire(smooth, 'value', target),
      ]);
    try {
      layer.setGraph(
        drivers: [...layer.getGraphDrivers(), level, remap, smooth],
        wiring: BridgeGraphWiring(
          edges: edges,
          // No positions passed for the new boxes: the Graph panel auto-places
          // what has no entry, exactly as it does for a fresh layer.
          layout: graph.wiring.layout,
          exposed: graph.wiring.exposed,
          groups: graph.wiring.groups,
          // Ducking wires a driver in; it does not touch whether the
          // layer's own output is plugged in (K-738).
          outUnwired: graph.wiring.outUnwired,
        ),
      );
    } catch (_) {
      _sayChainRefused();
      return;
    }
    widget.onChanged();
  }

  /// The one word a refused template gets.
  ///
  /// Reading the graph, minting the three drivers and committing the wiring can
  /// each refuse — a layer that has gone, a driver name this build does not
  /// know, wiring the engine will not take — and all three used to `return`
  /// without a sound, so the button read as dead. Which of the three it was is
  /// not something the user can act on differently, and a `BridgeError` reaches
  /// Dart as an opaque handle anyway, so they share a sentence.
  void _sayChainRefused() {
    if (!mounted) return;
    Provider.of<LumitState>(context, listen: false)
        .postNotice(l10n.audioChainNotBuilt);
  }
}
