// The Audio panel (docs/09, the approved AudioWorkspace board), down a column.
//
// **The output's levels**, the stereo bars off the engine's tap with their scale
// under them. **The picked layer's sound**: Volume and Pan with their stopwatches,
// the fade wells with their curve buttons, and the two graph-template buttons,
// whose staged chains are ordinary graph wires the Graph panel draws and the user
// can retune or delete. **Beats**, the face of the beat engine (docs/09 §5): one
// press to generate at rest, and its source, sensitivity, range, spacing and tempo
// a twirl away.
//
// The bars animate at UI rate inside their own RepaintBoundary off the shared
// meter feed; everything else rebuilds only when the document or the
// selection moves (the rebuild gates).

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/beats.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/beats_notice.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'audio_meters_feed.dart';
import 'audio_routes_frb.dart';
import 'clip_fade_popover.dart' show FadeCurveGlyph;
import 'graph_maths.dart' show rationalSeconds;
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'mixer_panel_frb.dart' show PanPot, meterRows;
import 'placeholder.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'timeline_outline_row_frb.dart' show RowRenameField;

/// The fade commands' silence, the engine's own floor
/// (`lumit_audio::mix::VOLUME_FLOOR_DB`).
const double _fadeFloorDb = -100;

/// A fade the fade commands wrote at one end of a layer: the index of its
/// first key, how long it runs, its shape, and the level it rises to.
typedef AudioFade = ({
  int first,
  double seconds,
  BridgeFadeShape shape,
  double level,
});

/// The fade at the layer's in point ([rising]) or out point, read off its
/// Volume keys, or null when that end has none.
///
/// A fade is two neighbouring keys, silence on the edge and the level on the
/// inside, which is all the fade commands write. The shape is read back from
/// the two sides they set: straight at both ends is Linear, eased only at the
/// loud end is Exponential, and anything else Ease. [slack] is how far from
/// the edge the silent key may sit, since the edge is read in whole frames.
AudioFade? audioFadeOf(
  BridgeScalar volume, {
  required double edgeSeconds,
  required bool rising,
  required double slack,
}) {
  if (volume is! BridgeScalar_Keyframed) return null;
  final keys = volume.field0;
  for (var i = 0; i + 1 < keys.length; i++) {
    final quiet = rising ? keys[i] : keys[i + 1];
    final loud = rising ? keys[i + 1] : keys[i];
    if ((rationalSeconds(quiet.time) - edgeSeconds).abs() > slack) continue;
    if (quiet.value > _fadeFloorDb + 0.1 || loud.value <= quiet.value) {
      continue;
    }
    final quietStraight = (rising ? quiet.interpOut : quiet.interpIn)
        is BridgeSideInterp_Linear;
    final loudStraight =
        (rising ? loud.interpIn : loud.interpOut) is BridgeSideInterp_Linear;
    return (
      first: i,
      seconds:
          (rationalSeconds(keys[i + 1].time) - rationalSeconds(keys[i].time))
              .abs(),
      shape: quietStraight
          ? (loudStraight ? BridgeFadeShape.linear : BridgeFadeShape.exponential)
          : BridgeFadeShape.ease,
      level: loud.value,
    );
  }
  return null;
}

/// The panel's two columns: the words, then the well each row edits.
const double audioLabelWidth = 74;
const double audioValueWidth = 72;

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

  /// Beats twirled open. Shut to begin with: generating is one press, and what
  /// tunes it is wanted far less often.
  bool _beatsOpen = false;

  /// Whether the comp holds generated beat markers, so Clear only shows when it
  /// has something to clear. Asked once per document revision.
  bool _hasBeats = false;
  int? _beatsAskedAt;

  /// The picked layer's name while it is being renamed from the panel.
  TextEditingController? _rename;

  void _endRename(BridgeLayerEntry entry, {required bool commit}) {
    final text = _rename?.text.trim();
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    if (commit && text != null && text.isNotEmpty && text != entry.info.name) {
      entry.layer.rename(name: text);
      Provider.of<LumitUiState>(context, listen: false).model.refresh();
    }
  }

  /// Which layers can make a sound, probed once and remembered.
  final Map<String, bool> _hasAudio = {};

  /// Whether the panel has been on screen a frame. The selected layer's rows
  /// come up into place when the pick changes, and are simply there when the
  /// panel itself arrives.
  bool _settled = false;

  @override
  void initState() {
    super.initState();
    if (widget.feed == null) {
      final ui = Provider.of<LumitUiState>(context, listen: false);
      meterPeakHoldSeconds = ui.workspace.interface.peakHoldSeconds;
      _feed.start(wake: Listenable.merge([ui.playing, ui.playheadFrame]));
    }
    WidgetsBinding.instance.addPostFrameCallback((_) => _settled = true);
  }

  @override
  void dispose() {
    if (widget.feed == null) _feed.dispose();
    _rename?.dispose();
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
        final revision = ui.model.revision;
        if (revision == null || revision != _beatsAskedAt) {
          _beatsAskedAt = revision;
          _hasBeats = markersOf(comp).any((m) => m.isBeat);
        }
        return Container(
          color: t.surface0,
          child: ListView(
            children: [
              _LevelsBlock(feed: _feed, ui: ui),
              _hair(t),
              // The pick is its own notifier: the model does not move when
              // a layer is picked, so this listens to it directly.
              ValueListenableBuilder<List<LayerReference>>(
                valueListenable: ui.selectedLayers,
                builder: (context, _, __) =>
                    _selectedLayer(context, t, ui, comp, sounding),
              ),
              _beatsHeading(context, t, ui, comp),
              if (_beatsOpen) _beats(context, t, ui, comp, sounding),
            ],
          ),
        );
      },
    );
  }

  Widget _hair(LumitTheme t) => Container(height: 1, color: t.hairline);

  Widget _label(LumitTheme t, String text) => SizedBox(
        width: audioLabelWidth,
        child: Text(text, style: t.body.copyWith(color: t.textMuted)),
      );

  /// A dropdown sized to what it says, standing at the start of the value
  /// column rather than stretched across the panel with its words centred.
  Widget _dropdown(Widget dropdown) => Flexible(
        child: Align(alignment: Alignment.centerLeft, child: dropdown),
      );

  // --- Beats -------------------------------------------------------------

  /// Beats' heading: the twirl, the word and the one press most visits want.
  Widget _beatsHeading(BuildContext context, LumitTheme t, LumitUiState ui,
          CompositionReference comp) =>
      GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => setState(() => _beatsOpen = !_beatsOpen),
        onSecondaryTapUp: (e) => showMenuAt<void>(
          context: context,
          position: e.globalPosition,
          width: 200,
          rows: (close) => [
            MenuRow(
              key: const ValueKey<String>('beats-menu-generate'),
              onPressed: () {
                close(null);
                _generate(context, ui, comp);
              },
              child: Text(l10n.beatsGenerate),
            ),
            MenuRow(
              key: const ValueKey<String>('beats-menu-clear'),
              enabled: _hasBeats,
              onPressed: () {
                close(null);
                comp.clearBeatMarkers();
                ui.model.refresh();
              },
              child: MenuLabel(l10n.beatsClearGenerated, enabled: _hasBeats),
            ),
          ],
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(6, 6, 10, 6),
          child: Row(children: [
            HoverMark(
              builder: (context, lit) => SizedBox(
                width: 16,
                child: Center(
                  child: TwirlTurn(
                    open: _beatsOpen,
                    child: glyph.LumitIcon(
                      _beatsOpen ? LumitIcons.collapse : LumitIcons.expand,
                      size: iconSize,
                      colour: Color.lerp(t.textMuted, t.textPrimary, lit),
                    ),
                  ),
                ),
              ),
            ),
            const SizedBox(width: 2),
            Text(l10n.audioSectionBeats,
                style: t.body.copyWith(color: t.textSecondary)),
            const SizedBox(width: 8),
            Expanded(
              child: Align(
                alignment: Alignment.centerRight,
                child: HouseButton(
                  key: const ValueKey('beats-generate'),
                  small: true,
                  onPressed: () => _generate(context, ui, comp),
                  child: Text(l10n.beatsGenerate,
                      maxLines: 1,
                      softWrap: false,
                      overflow: TextOverflow.ellipsis),
                ),
              ),
            ),
          ]),
        ),
      );

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
            _dropdown(
              BareDropdown<String>(
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
            _dropdown(
              BareDropdown<BeatRange>(
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
              width: audioValueWidth,
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
          // folds a line rather than painting outside its box.
          Wrap(
            spacing: 6,
            runSpacing: 4,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Row(mainAxisSize: MainAxisSize.min, children: [
                _label(t, l10n.beatsBpm),
                SizedBox(
                  width: audioValueWidth,
                  child: DragValueField(
                    key: const ValueKey('beats-bpm'),
                    value: bpmShown,
                    min: 0,
                    max: 300,
                    decimals: 2,
                    onChanged: (v) => setState(
                        () => _bpmOverride = v.toDouble().clamp(0, 300)),
                  ),
                ),
              ]),
              HouseButton(
                key: const ValueKey('beats-tap'),
                small: true,
                onPressed: _tap,
                child: Text(l10n.beatsTap),
              ),
              // The phase chips: the grid is right but early or late, which
              // only means anything once there is a grid. Ten milliseconds a
              // press, under half a frame at 60.
              if (bpmShown > 0) ...[
                _phaseChip(t, '‹', -10),
                _phaseChip(t, '›', 10),
              ],
              // Where the nudges have put the grid, so a press is seen to do
              // something before Generate is run again.
              if (_phaseMs != 0)
                Text(
                    '${_phaseMs > 0 ? '+' : ''}${_phaseMs.round()} ms',
                    key: const ValueKey('beats-phase'),
                    style: t.caption),
              if (bpmShown == 0) Text(l10n.beatsBpmAuto, style: t.caption),
            ],
          ),
          if (_hasBeats) ...[
            const SizedBox(height: 8),
            Align(
              alignment: Alignment.centerLeft,
              child: HouseButton(
                key: const ValueKey('beats-clear'),
                small: true,
                onPressed: () {
                  comp.clearBeatMarkers();
                  ui.model.refresh();
                },
                child: Text(l10n.beatsClearGenerated),
              ),
            ),
          ],
        ],
      ),
    );
  }

  Widget _phaseChip(LumitTheme t, String glyphText, double deltaMs) =>
      HoverMark(
        builder: (context, lit) => GestureDetector(
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
                  style: t.mono.copyWith(
                      fontSize: 10,
                      color:
                          Color.lerp(t.textSecondary, t.textPrimary, lit))),
            ),
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
    // The card, the bar and the sentences are the shared runner's
    // (state/beats_notice.dart) — the same ones the Timeline and the
    // Composition menu show. Only the refusal for a named layer is the panel's
    // own: a mute or a solo cannot silence a layer picked by name, so blaming
    // them would send the reader to the wrong switch.
    runBeatDetection(
      app: context.read<LumitState>(),
      comp: comp,
      options: options,
      onFound: (found) {
        if (mounted) setState(() => _lastBpm = found.bpm);
        ui.model.refresh();
      },
      noSound: _source.isEmpty ? null : l10n.beatsLayerNoSound,
    );
  }

  // --- Selected layer -----------------------------------------------------

  Widget _selectedLayer(
    BuildContext context,
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp,
    List<BridgeLayerEntry> sounding,
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
    // Nothing picked, nothing to show: the column closes up rather than saying
    // so.
    if (entry == null) return const SizedBox.shrink();
    final picked = entry;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapUp: (e) => showMenuAt<void>(
            context: context,
            position: e.globalPosition,
            width: 200,
            rows: (close) => audioLayerRows(
              theme: t,
              comp: comp,
              app: context.read<LumitState>(),
              entry: picked,
              targets: audioMenuTargets(picked, sounding, selected),
              buses: [
                for (final l in ui.model.heldLayers)
                  if (l.info.kind == BridgeLayerKind.bus) l,
              ],
              frame: ui.playheadFrame.value,
              closeParent: () => close(null),
              onRename: () => setState(() => _rename =
                  TextEditingController(text: picked.info.name)),
              onChanged: () {
                ui.model.refresh();
                comp.audioPrepare();
              },
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(10, 8, 10, 6),
            child: Row(children: [
              Container(
                width: 8,
                height: 8,
                decoration: BoxDecoration(
                  color: t.labelColour(picked.info.label),
                  borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                ),
              ),
              const SizedBox(width: 6),
              Expanded(
                child: _rename != null
                    ? RowRenameField(
                        controller: _rename!,
                        onCommit: () => _endRename(picked, commit: true),
                        onCancel: () => _endRename(picked, commit: false),
                      )
                    : Text(
                        picked.info.name,
                        style: t.bodyPrimary,
                        maxLines: 1,
                        softWrap: false,
                        overflow: TextOverflow.ellipsis,
                      ),
              ),
            ]),
          ),
        ),
        Entrance(
          key: ValueKey('audio-selected-in-${entry.layer.internallayerId}'),
          spec: _settled
              ? ThemeScope.of(context).motion.reveal
              : MotionSpec.still,
          rise: ThemeScope.of(context).motion.revealRise,
          child: _SelectedLayerBlock(
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
        ),
        _hair(t),
      ],
    );
  }
}

/// The output's two bars and their scale, painted as one inside one
/// RepaintBoundary, and the limiter's lamp, which shows only while lit and is its
/// own tiny widget under the clip listenable.
class _LevelsBlock extends StatelessWidget {
  final AudioMeterFeed feed;
  final LumitUiState ui;

  const _LevelsBlock({required this.feed, required this.ui});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (e) => showMenuAt<void>(
        context: context,
        position: e.globalPosition,
        width: 170,
        rows: (close) => meterRows(ui, () => close(null)),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(10, 10, 10, 8),
        child: Stack(
          children: [
            RepaintBoundary(
              key: const ValueKey('levels-bars'),
              child: SizedBox(
                height: LevelBarsPainter.height,
                width: double.infinity,
                child: CustomPaint(
                  painter: LevelBarsPainter(
                    frame: feed.frame,
                    trough: t.surface2,
                    edge: t.hairline,
                    body: t.success,
                    cap: t.warning,
                    hold: t.textPrimary,
                    text: t.mono.copyWith(fontSize: 9, color: t.textMuted),
                  ),
                ),
              ),
            ),
            Positioned(
              right: 0,
              bottom: 0,
              child: ValueListenableBuilder<bool>(
                valueListenable: feed.clipped,
                builder: (context, lit, _) => IgnorePointer(
                  ignoring: !lit,
                  child: Opacity(
                    opacity: lit ? 1 : 0,
                    child: LumitTooltip(
                      message: l10n.mixerLimiterClipped,
                      child: HoverMark(
                        builder: (context, over) => GestureDetector(
                          key: const ValueKey('audio-clip-lamp'),
                          onTap: resetMeterPeaks,
                          child: Container(
                            width: 8,
                            height: 8,
                            decoration: BoxDecoration(
                              color: Color.lerp(
                                  t.error, t.textPrimary, 0.35 * over),
                              shape: BoxShape.circle,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The two horizontal bars with their dB numbers and the scale under them,
/// painted as one so a tick repaints one boundary and rebuilds nothing.
class LevelBarsPainter extends CustomPainter {
  /// How tall the block paints: the two bars and the scale.
  static const double height = 36;
  static const double _barHeight = 6;
  static const double _rowStep = 12;
  static const double _leftGutter = 12;
  static const double _rightGutter = 44;
  static const List<int> _scale = [-48, -36, -24, -12, 0];

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

  void _word(Canvas canvas, String words, Offset at,
      {TextAlign align = TextAlign.left, double width = 0}) {
    final painter = TextPainter(
      text: TextSpan(text: words, style: text),
      textDirection: TextDirection.ltr,
      textAlign: align,
    )..layout(minWidth: width);
    painter.paint(canvas, at);
  }

  @override
  void paint(Canvas canvas, Size size) {
    final master = frame.value.master;
    _bar(canvas, size, 0, 'L', master.rmsLeft, master.peakLeft, master.holdLeft);
    _bar(canvas, size, 1, 'R', master.rmsRight, master.peakRight,
        master.holdRight);
    final width = size.width - _leftGutter - _rightGutter;
    final tickTop = _rowStep * 2 - 2;
    for (final db in _scale) {
      final x = _leftGutter + width * (db + 60) / 60;
      canvas.drawRect(
          Rect.fromLTWH(x - 0.5, tickTop, 1, 2), Paint()..color = text.color!);
      final painter = TextPainter(
        text: TextSpan(text: '${db.abs()}', style: text),
        textDirection: TextDirection.ltr,
      )..layout();
      painter.paint(canvas, Offset(x - painter.width / 2, tickTop + 3));
    }
  }

  void _bar(Canvas canvas, Size size, int row, String side, double rms,
      double peak, double holdValue) {
    final width = size.width - _leftGutter - _rightGutter;
    final y = row * _rowStep + 2;
    _word(canvas, side, Offset(0, row * _rowStep));
    canvas.save();
    canvas.translate(_leftGutter, 0);
    final rect = Rect.fromLTWH(0, y, width, _barHeight);
    canvas.drawRect(rect, Paint()..color = trough);
    canvas.drawRect(
      Rect.fromLTWH(0, y, width * meterFraction(peak), _barHeight),
      Paint()..color = cap.withValues(alpha: 0.55),
    );
    canvas.drawRect(
      Rect.fromLTWH(0, y, width * meterFraction(rms), _barHeight),
      Paint()..color = body,
    );
    if (holdValue > 0) {
      canvas.drawRect(
        Rect.fromLTWH(width * meterFraction(holdValue) - 1, y, 1, _barHeight),
        Paint()..color = hold,
      );
    }
    canvas.restore();
    final db = amplitudeDb(peak);
    _word(
      canvas,
      db <= -60 ? l10n.volumeNegInf : db.toStringAsFixed(1),
      Offset(size.width - _rightGutter + 4, row * _rowStep),
      align: TextAlign.right,
      width: _rightGutter - 4,
    );
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

  /// Whether the layer can make a sound. Off, the Volume, Pan and
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
  /// How long a fade is made by a curve button, at an end that has none yet.
  static const double _newFadeSeconds = 0.5;

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
        // The fades the layer already has, read off its Volume keys, so the
        // wells and chips say what is there rather than what was last typed.
        final fps = ui.model.fps;
        final slack = fps > 0 ? 0.5 / fps + 0.002 : 0.002;
        final fadeIn = fps <= 0
            ? null
            : audioFadeOf(volume,
                edgeSeconds: info.inFrame.toInt() / fps,
                rising: true,
                slack: slack);
        final fadeOut = fps <= 0
            ? null
            : audioFadeOf(volume,
                edgeSeconds: info.outFrame.toInt() / fps,
                rising: false,
                slack: slack);
        return Padding(
          padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (widget.canSound) ...[
              Wrap(
                  runSpacing: 4,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                SizedBox(
                  width: audioLabelWidth,
                  child: Text(l10n.volume,
                      style: t.body.copyWith(color: t.textMuted)),
                ),
                SizedBox(
                  width: audioValueWidth,
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
                const SizedBox(width: 6),
                KeyframeControlsFrb(
                  scalars: [volume],
                  comp: widget.comp,
                  onSeek: ui.scrubTo,
                  rowKey: 'audio-volume',
                  onWrite: (next) {
                    layer.setVolumeDb(value: next.single);
                    widget.onChanged();
                  },
                ),
              ]),
              const SizedBox(height: 6),
              Wrap(
                  runSpacing: 4,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                SizedBox(
                  width: audioLabelWidth,
                  child: Text(l10n.audioPan,
                      style: t.body.copyWith(color: t.textMuted)),
                ),
                SizedBox(
                  width: audioValueWidth,
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
                // The stopwatch first, so it stands under Volume's.
                KeyframeControlsFrb(
                  scalars: [pan],
                  comp: widget.comp,
                  onSeek: ui.scrubTo,
                  rowKey: 'audio-pan',
                  onWrite: (next) {
                    layer.setPan(value: next.single);
                    widget.onChanged();
                  },
                ),
                const SizedBox(width: 6),
                // The board's own dial beside the value well — the Mixer
                // strip's pot, turning the same way.
                PanPot(
                  key: const ValueKey('audio-pan-pot'),
                  value: panValue,
                  onLive: (v) => setState(() => _dragPan = v),
                  onCommit: (v) => _commitPan(pan, v, frame),
                  onCancel: () => setState(() => _dragPan = null),
                ),
              ]),
              const SizedBox(height: 6),
              _fadeRow(
                t,
                key: 'fade-in',
                label: l10n.audioFadeIn,
                rising: true,
                fade: fadeIn,
              ),
              const SizedBox(height: 6),
              _fadeRow(
                t,
                key: 'fade-out',
                label: l10n.audioFadeOut,
                rising: false,
                fade: fadeOut,
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

  /// A fade well and its three curve buttons. The well holds the length, or
  /// nought at an end with no fade, and a button the shape, and either writes
  /// the fade there and then. A button is lit only while the layer has a fade at
  /// that end, and a length of 0 takes the fade away.
  Widget _fadeRow(
    LumitTheme t, {
    required String key,
    required String label,
    required bool rising,
    required AudioFade? fade,
  }) {
    final shape = fade?.shape;
    final seconds = fade?.seconds ?? 0.0;
    Widget chip(String name, BridgeFadeShape option,
            double Function(double) gain) =>
        LumitTooltip(
          message: name,
          child: HoverMark(
            builder: (context, lit) => GestureDetector(
              key: ValueKey('$key-${option.name}'),
              behavior: HitTestBehavior.opaque,
              onTap: () => _writeFade(
                  rising: rising,
                  fade: fade,
                  seconds: fade == null ? _newFadeSeconds : seconds,
                  shape: option),
              child: Container(
                width: 22,
                height: 18,
                alignment: Alignment.center,
                color: shape == option
                    ? t.accent.withValues(alpha: 0.18)
                    : t.accent.withValues(alpha: 0),
                child: FadeCurveGlyph(
                  gain: gain,
                  falling: !rising,
                  colour: shape == option
                      ? t.textPrimary
                      : Color.lerp(t.textMuted, t.textPrimary, lit),
                ),
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
          width: audioLabelWidth,
          child: Text(label, style: t.body.copyWith(color: t.textMuted)),
        ),
        SizedBox(
          width: audioValueWidth,
          child: DragValueField(
            key: ValueKey('$key-seconds'),
            value: seconds,
            min: 0,
            max: 60,
            decimals: 2,
            suffix: ' s',
            speed: 0.02,
            onChanged: (v) => _writeFade(
                rising: rising,
                fade: fade,
                seconds: v.toDouble(),
                shape: shape ?? BridgeFadeShape.ease),
          ),
        ),
        const SizedBox(width: 6),
        // One group, with the hairline round all three.
        Container(
          decoration: BoxDecoration(
            border: Border.all(color: t.hairline),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          clipBehavior: Clip.antiAlias,
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            chip(l10n.fadeShapeEase, BridgeFadeShape.ease,
                (u) => u * u * (3 - 2 * u)),
            chip(l10n.fadeShapeLinear, BridgeFadeShape.linear, (u) => u),
            chip(l10n.fadeShapeExponential, BridgeFadeShape.exponential,
                (u) => 1 - (1 - u) * (1 - u) * (1 - u)),
          ]),
        ),
      ],
    );
  }

  /// Write a fade at one end, as one undo step. The old fade's pair of keys
  /// comes off first, so a shorter fade does not leave the old far key behind
  /// and the new one rises to the layer's real level rather than to a point
  /// halfway up the old ramp.
  void _writeFade({
    required bool rising,
    required AudioFade? fade,
    required double seconds,
    required BridgeFadeShape shape,
  }) {
    final layer = widget.entry.layer;
    final volume = widget.entry.info.volumeDb;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      try {
        if (fade != null && volume is BridgeScalar_Keyframed) {
          final rest = [...volume.field0]..removeRange(fade.first, fade.first + 2);
          layer.setVolumeDb(
              value: rest.isEmpty
                  ? BridgeScalar.static_(fade.level)
                  : BridgeScalar.keyframed(rest));
        }
        if (seconds > 0) {
          rising
              ? layer.fadeIn(seconds: seconds, shape: shape)
              : layer.fadeOut(seconds: seconds, shape: shape);
        }
      } catch (_) {
        // A fade longer than the layer, or a locked layer: nothing written.
      }
    });
    widget.onChanged();
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

  // --- The two graph templates -------------------------------------------

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

  /// *Lower behind…* (the ducking template, in plain words): pick the layer
  /// whose sound pushes this one down, then stage the inverted chain onto
  /// this layer's own Volume socket — Audio level listening to the picked
  /// layer, Remap upside down, Smooth, into the Layer out's Volume.
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

  /// Audio level's Source row, the entry that reads one named layer. A number
  /// because a Choice row crosses the bridge as its index.
  static const int _sourceLayer = 1;

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
      // Naming a layer is half of it: the Source row is what says which of the
      // node's three readings it takes (docs/impl/audio-nodes.md §3), and it
      // starts on the whole composition.
      level.setValue(
          id: 'source', value: const BridgeEffectValue.choice(_sourceLayer));
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
          // layer's own output is plugged in.
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
