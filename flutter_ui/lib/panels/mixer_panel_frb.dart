// The Mixer panel (docs/09 §3.1, the approved AudioWorkspace
// board): one strip per audible row of the fronted comp, in comp order, and
// the Master strip at the right-hand edge.
//
// Each strip is the board's drawing: the layer's name underlined in its label
// colour, a pan pot, a fader beside the stereo meters, the dB well, and the
// mute, solo and fx switches. The Master strip swaps the pot for the limiter
// lamp and carries the loudness beside its meter. Every part of a strip
// answers a right-click, and all the strips' faders stand level, whichever of
// them have something extra to show.
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
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show insertEmptyLayer;
import '../shell/number_dialog_frb.dart';
import '../state/app_state.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/edge_fade.dart';
import 'audio_loudness.dart';
import 'audio_meters_feed.dart';
import 'audio_routes_frb.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'placeholder.dart';
import 'timeline_extras_frb.dart' show rowSelectionFill, showMenuAt;
import 'timeline_outline_row_frb.dart' show RowRenameField;

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

/// The menu rows every level meter wears: how long its hold line rests,
/// putting the lines and the limiter's light out, and starting the loudness
/// afresh.
List<Widget> meterRows(LumitUiState ui, VoidCallback close) {
  final settings = ui.workspace.interface;
  return [
    SubmenuRow(
      key: const ValueKey<String>('meter-menu-hold'),
      closeParent: close,
      submenu: (dismiss) => FloatSurface(
        width: 130,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final (seconds, label) in [
              (0, l10n.mixerPeakHoldOff),
              (3, l10n.mixerPeakHoldSeconds(3)),
              (10, l10n.mixerPeakHoldSeconds(10)),
              (-1, l10n.mixerPeakHoldForever),
            ])
              MenuRow(
                key: ValueKey<String>('meter-menu-hold-$seconds'),
                onPressed: () {
                  dismiss();
                  settings.peakHoldSeconds = seconds;
                  meterPeakHoldSeconds = seconds;
                  ui.workspace.settingsChanged();
                  resetMeterPeaks();
                },
                child:
                    MenuLabel(label, ticked: settings.peakHoldSeconds == seconds),
              ),
          ],
        ),
      ),
      child: Text(l10n.mixerPeakHold),
    ),
    MenuRow(
      key: const ValueKey<String>('meter-menu-reset'),
      onPressed: () {
        close();
        resetMeterPeaks();
      },
      child: Text(l10n.mixerResetPeaks),
    ),
    MenuRow(
      key: const ValueKey<String>('meter-menu-reset-loudness'),
      onPressed: () {
        close();
        resetLoudness();
      },
      child: Text(l10n.mixerResetLoudness),
    ),
  ];
}

/// [scalar] with a key added at [frame], or the one there taken away. Taking the
/// last one away leaves the plain value it held.
BridgeScalar scalarWithKeyToggled(
    BridgeScalar scalar, double value, CompositionReference comp, int frame) {
  final keys = scalar is BridgeScalar_Keyframed
      ? scalar.field0
      : const <BridgeKeyframe>[];
  if (keys.any((k) => comp.frameAtTime(time: k.time) == frame)) {
    final rest = [
      for (final k in keys)
        if (comp.frameAtTime(time: k.time) != frame) k,
    ];
    return rest.isEmpty
        ? BridgeScalar.static_(value)
        : BridgeScalar.keyframed(rest);
  }
  if (keys.isNotEmpty) return scalarWithValueAt(scalar, value, comp, frame);
  return BridgeScalar.keyframed([
    BridgeKeyframe(
      time: timeOfFrame(comp, frame),
      value: value,
      interpIn: const BridgeSideInterp.linear(),
      interpOut: const BridgeSideInterp.linear(),
    ),
  ]);
}

/// The rows a track's Volume, Pan or Level gives wherever it is pressed, the
/// Mixer's fader and pot and the Cut timeline's head bars alike: back to its
/// usual value, a number asked for, any sides it can be thrown to, and a key at
/// the playhead.
List<Widget> mixValueRows({
  required BuildContext context,
  required CompositionReference comp,
  required int frame,
  required BridgeScalar scalar,
  required void Function(BridgeScalar next) write,
  required VoidCallback onChanged,
  required VoidCallback close,
  required String resetLabel,
  required String askLabel,
  required String title,
  required double min,
  required double max,
  required int decimals,
  double reset = 0,
  String? suffix,
  List<(String, double)> sides = const [],
  bool tickColumn = false,
}) {
  // Room for a tick, when a row the caller adds below wears one.
  final tick = tickColumn ? false : null;
  final value = scalar is BridgeScalar_Static
      ? scalar.field0
      : sampledScalar(scalar, timeOfFrame(comp, frame));
  final keyed = scalar is BridgeScalar_Keyframed &&
      scalar.field0.any((k) => comp.frameAtTime(time: k.time) == frame);
  void put(double v) {
    try {
      write(scalarWithValueAt(scalar, v, comp, frame));
    } catch (_) {
      // A locked layer refuses, and quietly.
      return;
    }
    onChanged();
  }

  return [
    MenuRow(
      key: const ValueKey<String>('mixer-value-reset'),
      enabled: value != reset,
      onPressed: () {
        close();
        put(reset);
      },
      child: MenuLabel(resetLabel, enabled: value != reset, ticked: tick),
    ),
    MenuRow(
      key: const ValueKey<String>('mixer-value-ask'),
      onPressed: () async {
        close();
        final typed = await askNumberFrb(
            context: context,
            title: title,
            value: value,
            min: min,
            max: max,
            decimals: decimals,
            suffix: suffix);
        if (typed != null && context.mounted) put(typed);
      },
      child: MenuLabel(askLabel, ticked: tick),
    ),
    for (final (label, side) in sides)
      MenuRow(
        key: ValueKey<String>('mixer-value-side-$side'),
        onPressed: () {
          close();
          put(side);
        },
        child: MenuLabel(label, ticked: tick),
      ),
    // A value an expression drives has no keys to add.
    if (scalar is! BridgeScalar_Expression) ...[
      const MenuDivider(),
      MenuRow(
        key: const ValueKey<String>('mixer-value-key'),
        onPressed: () {
          close();
          try {
            write(scalarWithKeyToggled(scalar, value, comp, frame));
          } catch (_) {
            return;
          }
          onChanged();
        },
        child: MenuLabel(keyed ? l10n.tipRemoveKeyframe : l10n.tipAddKeyframe,
            ticked: tick),
      ),
    ],
  ];
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
    if (widget.feed == null) {
      final ui = Provider.of<LumitUiState>(context, listen: false);
      meterPeakHoldSeconds = ui.workspace.interface.peakHoldSeconds;
      _feed.start(wake: Listenable.merge([ui.playing, ui.playheadFrame]));
    }
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

  /// The desk's own ground, off every strip: new strips, and what can be said
  /// of all of them at once.
  void _groundMenu(LumitUiState ui, CompositionReference comp, Offset at,
      List<BridgeLayerEntry> sounding) {
    final muted = [
      for (final e in sounding)
        if (!e.info.switches.audible) e.layer.internallayerId,
    ];
    final soloed = [
      for (final e in sounding)
        if (e.info.switches.solo) e.layer.internallayerId,
    ];
    final raised = [
      for (final e in sounding)
        if (levelMoved(e.info.volumeDb, comp, ui.playheadFrame.value)) e,
    ];
    void changed() {
      ui.model.refresh();
      comp.audioPrepare();
    }

    void flip(BridgeLayerSwitch which, List<UuidValue> layers, bool on) {
      try {
        comp.setSwitchOnLayers(
            clicked: layers.first, layers: layers, switch_: which, on_: on);
      } catch (_) {}
      changed();
    }

    showMenuAt<void>(
      context: context,
      position: at,
      width: 210,
      rows: (close) => [
        for (final (action, label) in [
          ('layer.new.audio', l10n.insertAudioLayer),
          ('layer.new.bus', l10n.insertBus),
        ])
          MenuRow(
            key: ValueKey<String>('mixer-$action'),
            onPressed: () {
              close(null);
              insertEmptyLayer(ui, comp, action);
              changed();
            },
            child: MenuLabel(label, action: action),
          ),
        if (sounding.isNotEmpty) ...[
          const MenuDivider(),
          MenuRow(
            key: const ValueKey<String>('mixer-unmute-all'),
            enabled: muted.isNotEmpty,
            onPressed: () {
              close(null);
              flip(BridgeLayerSwitch.audible, muted, true);
            },
            child: MenuLabel(l10n.mixerUnmuteAll, enabled: muted.isNotEmpty),
          ),
          MenuRow(
            key: const ValueKey<String>('mixer-clear-solos'),
            enabled: soloed.isNotEmpty,
            onPressed: () {
              close(null);
              flip(BridgeLayerSwitch.solo, soloed, false);
            },
            child: MenuLabel(l10n.mixerClearSolos, enabled: soloed.isNotEmpty),
          ),
          MenuRow(
            key: const ValueKey<String>('mixer-reset-faders'),
            enabled: raised.isNotEmpty,
            onPressed: () {
              close(null);
              asOneUndoStep(
                  Provider.of<LumitState>(context, listen: false).project, () {
                for (final e in raised) {
                  try {
                    e.layer.setVolumeDb(
                        value: scalarWithValueAt(
                            e.info.volumeDb, 0, comp, ui.playheadFrame.value));
                  } catch (_) {
                    // A locked layer refuses, and the rest still take it.
                  }
                }
              });
              changed();
            },
            child: MenuLabel(l10n.mixerResetFaders, enabled: raised.isNotEmpty),
          ),
        ],
      ],
    );
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
        // A line under the names is what a routed strip's sound says, and every
        // strip keeps the room for it while any of them is, so the faders stand
        // level. While none is, it takes up nothing.
        final routed = sounding.any((e) => e.info.output != null);
        // The Master's loudness takes room beside its meter only while that
        // costs the desk no strip.
        return LayoutBuilder(
          builder: (context, box) => Container(
          color: t.surface0,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(
                child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onSecondaryTapUp: (e) =>
                    _groundMenu(ui, comp, e.globalPosition, sounding),
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
                    // panel shows, and where they are. The strips stop a gap
                    // short of the Master, so a strip cut at the edge fades
                    // out before its scale meets the Master's.
                    : Padding(
                        padding: const EdgeInsets.only(right: _deskGap),
                        child: Listener(
                        onPointerSignal: _wheel,
                        child: LayoutBuilder(
                        builder: (context, box) {
                          final width =
                              _stripWidthIn(box.maxWidth, sounding.length);
                          return HouseScrollbar(
                        controller: _strips,
                        crossAxisMargin: 2,
                        // Shown for as long as strips are out of sight.
                        thumbVisibility:
                            width * sounding.length > box.maxWidth + 0.5,
                        child: EdgeFade(
                        reach: _fadeReach,
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
                                desk: sounding,
                                width: width,
                                routed: routed,
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
                      );
                        },
                      ),
                      ),
                      ),
                ),
              ),
              _MasterStrip(
                comp: comp,
                feed: _feed,
                routed: routed,
                loudness: _wholeStrips(
                        box.maxWidth - _stripWidth - _loudnessWidth,
                        sounding.length) ==
                    _wholeStrips(box.maxWidth - _stripWidth, sounding.length),
              ),
            ],
          ),
          ),
        );
      },
    );
  }
}

/// The width the board draws a strip at, and the most one takes when the desk
/// has room for its name.
const double _stripWidth = 62;
const double _stripWidthMost = 96;

/// The column beside the Master strip's meter that holds its loudness, and
/// how much wider than a strip that makes the Master.
const double _loudnessColumn = 28;
const double _loudnessWidth = 13;

/// The desk's own ground kept beside strips that all fit, for its right-click.
const double _deskGround = 16;

/// The gap between the strips and the Master, and how far in from a cut edge
/// the strips fade: most of a part strip, so its numbers never read.
const double _deskGap = 10;
const double _fadeReach = 34;

/// How wide each of [count] strips stands on a desk [room] wide. They share
/// the room, so a spare inch goes to the names. A desk too narrow for all of
/// them shows whole strips and a third of the next under the fade, so it says
/// there are more.
double _stripWidthIn(double room, int count) {
  final whole = _wholeStrips(room, count);
  if (whole == count) {
    return ((room - _deskGround) / count).clamp(_stripWidth, _stripWidthMost);
  }
  return math.max(_stripWidth, room / (whole + 0.35));
}

/// How many of [count] strips stand whole on a desk [room] wide.
int _wholeStrips(double room, int count) => room ~/ _stripWidth >= count
    ? count
    : (room / _stripWidth - 0.35).floor().clamp(1, count);

/// How tall the strip's rows that must stand level across the desk are.
const double _nameRow = 18;
const double _routeRow = 12;
const double _switchRow = 14;

class _MixerStrip extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

  /// Every strip on the desk, which a menu on a picked strip acts across.
  final List<BridgeLayerEntry> desk;

  /// How wide the desk has this strip stand.
  final double width;

  /// Whether any strip on the desk is routed, so this one keeps a line for it.
  final bool routed;
  final AudioMeterFeed feed;
  final VoidCallback onChanged;

  const _MixerStrip({
    super.key,
    required this.comp,
    required this.entry,
    required this.desk,
    required this.width,
    required this.routed,
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

  /// The name's field while the strip is being renamed, null the rest of the
  /// time.
  TextEditingController? _rename;

  @override
  void dispose() {
    _rename?.dispose();
    super.dispose();
  }

  void _beginRename() {
    if (widget.entry.info.switches.locked) return;
    setState(
        () => _rename = TextEditingController(text: widget.entry.info.name));
  }

  void _endRename({required bool commit}) {
    final text = _rename?.text.trim();
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    if (commit &&
        text != null &&
        text.isNotEmpty &&
        text != widget.entry.info.name) {
      widget.entry.layer.rename(name: text);
      widget.onChanged();
    }
  }

  // ---------------------------------------------------------------- the menus

  /// The strip's own right-click: the same rows a track head has in the Audio
  /// timeline, since it is the same layer.
  void _stripMenu(Offset at) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    showMenuAt<void>(
      context: context,
      position: at,
      width: 200,
      rows: (close) => audioLayerRows(
        theme: ThemeScope.of(context).theme,
        comp: widget.comp,
        app: Provider.of<LumitState>(context, listen: false),
        entry: widget.entry,
        targets:
            audioMenuTargets(widget.entry, widget.desk, ui.selectedLayerIds),
        buses: [
          for (final l in ui.model.heldLayers)
            if (l.info.kind == BridgeLayerKind.bus) l,
        ],
        frame: ui.playheadFrame.value,
        closeParent: () => close(null),
        onRename: _beginRename,
        onChanged: widget.onChanged,
        renameAction: null,
        duplicateAction: null,
      ),
    );
  }

  /// A fader's or a pot's right-click, as [mixValueRows] gives it.
  void _valueMenu(
    Offset at, {
    required BridgeScalar scalar,
    required void Function(BridgeScalar next) write,
    required String resetLabel,
    required String askLabel,
    required String title,
    required double min,
    required double max,
    required int decimals,
    String? suffix,
    List<(String, double)> sides = const [],
  }) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    showMenuAt<void>(
      context: context,
      position: at,
      width: 190,
      rows: (close) => mixValueRows(
        context: context,
        comp: widget.comp,
        frame: ui.playheadFrame.value,
        scalar: scalar,
        write: write,
        onChanged: widget.onChanged,
        close: () => close(null),
        resetLabel: resetLabel,
        askLabel: askLabel,
        title: title,
        min: min,
        max: max,
        decimals: decimals,
        suffix: suffix,
        sides: sides,
      ),
    );
  }

  void _faderMenu(Offset at) => _valueMenu(
        at,
        scalar: widget.entry.info.volumeDb,
        write: (next) => widget.entry.layer.setVolumeDb(value: next),
        resetLabel: l10n.audioResetVolume,
        askLabel: l10n.mixerSetLevel,
        title: l10n.volume,
        min: _faderFloorDb,
        max: 12,
        decimals: 1,
        suffix: ' ${l10n.unitSymbolDb}',
      );

  void _potMenu(Offset at) => _valueMenu(
        at,
        scalar: widget.entry.info.pan,
        write: (next) => widget.entry.layer.setPan(value: next),
        resetLabel: l10n.audioResetPan,
        askLabel: l10n.mixerSetPan,
        title: l10n.audioPan,
        min: -100,
        max: 100,
        decimals: 0,
        sides: [(l10n.mixerPanLeft, -100), (l10n.mixerPanRight, 100)],
      );

  void _meterMenu(Offset at) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    showMenuAt<void>(
      context: context,
      position: at,
      width: 170,
      rows: (close) => meterRows(ui, () => close(null)),
    );
  }

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
      onPointerDown: (e) {
        final isIn = ui.selectedLayerIds.contains(layer.internallayerId);
        if (HardwareKeyboard.instance.isControlPressed ||
            HardwareKeyboard.instance.isShiftPressed) {
          ui.setSelection([
            for (final l in ui.selectedLayers.value)
              if (l.internallayerId != layer.internallayerId) l,
            if (!isIn) layer,
          ]);
        } else if (!isIn) {
          ui.setSelection([layer]);
        }
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onSecondaryTapUp: (e) => _stripMenu(e.globalPosition),
        child: SizedBox(
        width: widget.width,
        child: ValueListenableBuilder<List<LayerReference>>(
        valueListenable: ui.selectedLayers,
        builder: (context, picked, child) => AnimatedContainer(
          duration: mark.duration,
          curve: mark.curve,
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
        ),
      ),
    );
  }

  /// The strip's name, with its label colour under it, or the field it turns
  /// into. A double-click is the way in, as it is on a name anywhere.
  Widget _name(LumitTheme t, BridgeLayerInfo info) => SizedBox(
        height: _nameRow,
        child: _rename != null
            ? RowRenameField(
                key: const ValueKey<String>('strip-rename'),
                controller: _rename!,
                onCommit: () => _endRename(commit: true),
                onCancel: () => _endRename(commit: false),
              )
            : LumitTooltip(
                message: info.name,
                child: GestureDetector(
                  key: ValueKey<String>(
                      'strip-name-${widget.entry.layer.internallayerId}'),
                  behavior: HitTestBehavior.opaque,
                  onDoubleTap: _beginRename,
                  child: Container(
                    constraints: BoxConstraints(maxWidth: widget.width - 8),
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
                      style: t.small.copyWith(color: t.textPrimary),
                      maxLines: 1,
                      softWrap: false,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ),
              ),
      );

  /// Which bus this strip's sound goes to, while it does not go to the master.
  Widget _route(LumitTheme t, BridgeLayerInfo info) {
    String? bus;
    for (final l in Provider.of<LumitUiState>(context, listen: false)
        .model
        .heldLayers) {
      if (l.layer.internallayerId == info.output) bus = l.info.name;
    }
    return SizedBox(
      height: _routeRow,
      width: widget.width - 8,
      child: bus == null
          ? null
          : Text(
              l10n.mixerRoutedTo(bus),
              style: t.caption.copyWith(color: t.textMuted),
              maxLines: 1,
              softWrap: false,
              overflow: TextOverflow.ellipsis,
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
    final chain = info.effects.where((e) => e.enabled && e.audio).length;
    return Column(
      children: [
        _name(t, info),
        if (widget.routed) _route(t, info),
        const SizedBox(height: 6),
        GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapUp: (e) => _potMenu(e.globalPosition),
          child: PanPot(
            key: ValueKey('pot-$id'),
            value: panValue,
            onLive: (v) => setState(() => _dragPan = v),
            onCommit: (v) => _commitPan(pan, v, frame),
            onCancel: () => setState(() => _dragPan = null),
          ),
        ),
        const SizedBox(height: 4),
        Expanded(
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              GestureDetector(
                onSecondaryTapUp: (e) => _faderMenu(e.globalPosition),
                child: _Fader(
                  key: ValueKey('fader-$id'),
                  db: db,
                  onLive: (v) => setState(() => _dragDb = v),
                  onCommit: (v) => _commitVolume(volume, v, frame),
                  onCancel: () => setState(() => _dragDb = null),
                ),
              ),
              const SizedBox(width: 3),
              GestureDetector(
                behavior: HitTestBehavior.opaque,
                onSecondaryTapUp: (e) => _meterMenu(e.globalPosition),
                child: MeterBand(stripId: id, feed: widget.feed),
              ),
            ],
          ),
        ),
        const SizedBox(height: 4),
        _DbWell(
          db: db,
          onCommit: (v) => _commitVolume(volume, v, frame),
        ),
        const SizedBox(height: 4),
        SizedBox(
          height: _switchRow,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              _SwitchGlyph(
                keyName: 'mute-$id',
                mark: switches.audible ? LumitIcons.audio : LumitIcons.muted,
                on: switches.audible,
                tip: switches.audible ? l10n.switchAudible : l10n.switchMuted,
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
              // A strip with plugins in its sound says so and can bypass them,
              // which is the rack's one control a desk wants at hand. The stack
              // is the rack, and Effect controls is where it is worked.
              if (chain > 0) ...[
                const SizedBox(width: 5),
                _SwitchGlyph(
                  keyName: 'chain-$id',
                  icon: LumitIcon.fx,
                  on: switches.fx,
                  tip: l10n.mixerChain(chain),
                  onTap: () {
                    layer.setSwitch(
                        switch_: BridgeLayerSwitch.fx, on_: !switches.fx);
                    widget.onChanged();
                  },
                ),
              ],
            ],
          ),
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

/// The Master strip: the fader is a gain stage on the sum, and the lamp is the
/// limiter's sticky clip light.
class _MasterStrip extends StatefulWidget {
  final CompositionReference comp;
  final AudioMeterFeed feed;

  /// Whether the strips keep a line under their names, so this fader stands
  /// level with theirs.
  final bool routed;

  /// Whether the loudness has room to stand beside the meter, rather than
  /// the integrated number alone under the level.
  final bool loudness;

  const _MasterStrip(
      {required this.comp,
      required this.feed,
      required this.routed,
      required this.loudness});

  @override
  State<_MasterStrip> createState() => _MasterStripState();
}

class _MasterStripState extends State<_MasterStrip> {
  double? _dragDb;

  void _write(double db) {
    widget.comp.setMasterVolumeDb(db: db);
    setState(() => _dragDb = null);
    context.read<LumitUiState>().model.refresh();
    widget.comp.audioPrepare();
  }

  void _menu(Offset at) {
    final ui = context.read<LumitUiState>();
    final db = widget.comp.masterVolumeDb();
    showMenuAt<void>(
      context: context,
      position: at,
      width: 190,
      rows: (close) => [
        MenuRow(
          key: const ValueKey<String>('master-reset'),
          enabled: db != 0,
          onPressed: () {
            close(null);
            _write(0);
          },
          child: MenuLabel(l10n.audioResetVolume, enabled: db != 0),
        ),
        MenuRow(
          key: const ValueKey<String>('master-ask'),
          onPressed: () async {
            close(null);
            final typed = await askNumberFrb(
                context: context,
                title: l10n.mixerMaster,
                value: db,
                min: _faderFloorDb,
                max: 12,
                decimals: 1,
                suffix: ' dB');
            if (typed != null && mounted) _write(typed);
          },
          child: MenuLabel(l10n.mixerSetLevel),
        ),
        const MenuDivider(),
        ...meterRows(ui, () => close(null)),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final db = _dragDb ?? widget.comp.masterVolumeDb();
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (e) => _menu(e.globalPosition),
      child: Container(
        width: _stripWidth + (widget.loudness ? _loudnessWidth : 0),
        decoration: BoxDecoration(
          color: t.surface1,
          border: Border(left: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Column(
          children: [
            SizedBox(
              height: _nameRow,
              child: Text(
                l10n.mixerMaster,
                style: t.small.copyWith(color: t.textPrimary),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            if (widget.routed) const SizedBox(height: _routeRow),
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
                    onCommit: _write,
                    onCancel: () => setState(() => _dragDb = null),
                  ),
                  const SizedBox(width: 3),
                  MeterBand(stripId: '', feed: widget.feed),
                  // Beside the meter, its top level with the fader's.
                  if (widget.loudness) ...[
                    const SizedBox(width: 5),
                    SizedBox(
                      width: _loudnessColumn,
                      child: Align(
                        alignment: Alignment.topLeft,
                        child: LoudnessReadout(
                            feed: widget.feed, layout: LoudnessLayout.column),
                      ),
                    ),
                  ],
                ],
              ),
            ),
            const SizedBox(height: 4),
            _DbWell(db: db, onCommit: _write),
            const SizedBox(height: 4),
            // With no room beside the meter, the integrated loudness stands
            // where a strip has its switches.
            SizedBox(
              height: _switchRow,
              child: widget.loudness
                  ? null
                  : Center(
                      child: LoudnessReadout(
                          feed: widget.feed,
                          layout: LoudnessLayout.integrated),
                    ),
            ),
          ],
        ),
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
            onTap: resetMeterPeaks,
            child: AnimatedContainer(
              duration: ThemeScope.of(context).motion.mark.duration,
              width: 30,
              height: 14,
              alignment: Alignment.center,
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
                  // A fader at unity is where it usually rests, so it stays
                  // quiet, and one that has been moved takes the accent.
                  knob: Color.lerp(
                      widget.db == 0 ? t.textSecondary : t.accent,
                      t.textPrimary,
                      _glow.light * 0.3)!,
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
    // On a short strip the numbers would run into each other, so any too near
    // the last one printed gives way, working down from the top.
    var lastY = double.negativeInfinity;
    for (final (db, text) in _faderScale) {
      final y = yOf(_faderFraction(db));
      // Unity is the one a hand looks for, so its mark is the longer.
      final unity = db == 0;
      canvas.drawRect(
        Rect.fromLTWH(x - (unity ? 7 : 5), y - 0.5, unity ? 4 : 2, 1),
        Paint()..color = tick,
      );
      if (y - lastY < 12) continue;
      lastY = y;
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
              mark: Color.lerp(value == 0 ? t.textSecondary : t.accent,
                  t.textPrimary, lit * 0.3)!,
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

/// The dB well under a fader. It reads the level, and a click turns it into a
/// field that takes a number.
class _DbWell extends StatefulWidget {
  final double db;
  final ValueChanged<double> onCommit;

  const _DbWell({required this.db, required this.onCommit});

  @override
  State<_DbWell> createState() => _DbWellState();
}

class _DbWellState extends State<_DbWell> {
  bool _typing = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final shown = widget.db <= _faderFloorDb
        ? l10n.volumeNegInf
        : widget.db.toStringAsFixed(1);
    return Container(
      width: 54,
      height: 16,
      alignment: Alignment.centerRight,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      decoration: BoxDecoration(
        color: t.surface0,
        border: Border.all(color: _typing ? t.accent : t.hairline),
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
      ),
      child: _typing
          ? Focus(
              onFocusChange: (on) {
                if (!on) setState(() => _typing = false);
              },
              child: DragValueField(
                bare: true,
                autofocus: true,
                value: widget.db,
                min: _faderFloorDb,
                max: 12,
                decimals: 1,
                onChanged: (v) => widget.onCommit(v.toDouble()),
                onSubmitted: () => setState(() => _typing = false),
              ),
            )
          : MouseRegion(
              cursor: SystemMouseCursors.text,
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: () => setState(() => _typing = true),
                child: Align(
                  alignment: Alignment.centerRight,
                  child: Text(
                    '$shown ${l10n.unitSymbolDb}',
                    style: t.mono.copyWith(
                        fontSize: 9,
                        color: widget.db == 0 ? t.textMuted : t.textPrimary),
                    maxLines: 1,
                  ),
                ),
              ),
            ),
    );
  }
}

class _SwitchGlyph extends StatelessWidget {
  final String keyName;

  /// One of the set's glyphs, or one of the older marks through [icon].
  final String? mark;
  final LumitIcon? icon;
  final bool on;
  final String tip;
  final VoidCallback onTap;

  const _SwitchGlyph({
    required this.keyName,
    this.mark,
    this.icon,
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
            builder: (context, ink, _) {
              final colour = Color.lerp(ink, t.textPrimary, on ? lit : 0)!;
              return mark != null
                  ? glyph.LumitIcon(mark!, size: 12, colour: colour)
                  : lumitIcon(icon!, size: 12, color: colour);
            },
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
