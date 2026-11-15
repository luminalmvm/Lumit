// The Mixer panel (docs/09 §3.1): one strip per sounding layer of the
// fronted comp, in layer order, then the buses on a raised ground, and the
// Master strip at the right-hand edge.
//
// A strip is drawn in `mixer_strip.dart`, its rack in `mixer_rack.dart`, the
// Master in `mixer_master.dart` and the small controls in `mixer_meter.dart`.
// This file lays out the desk, keeps every strip's sections level, answers
// the desk's own right-click and the Mixer's keys.
//
// **The meters run off the feed.** One [AudioMeterFeed] polls the engine's
// tap and the meter painters listen to its frame directly, each inside its own
// RepaintBoundary: a tick repaints the bars and rebuilds no widget at all,
// which is what keeps the redraw gates green (docs/impl/ui-performance.md
// WP-2). Everything else draws from the held comp model and rebuilds only when
// the document moves.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart' show BridgeKeyContext;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show insertEmptyLayer;
import '../shell/number_dialog_frb.dart';
import '../state/app_state.dart';
import '../state/comp_time.dart';
import '../state/dock.dart' show Panel;
import '../state/ui_state.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/edge_fade.dart';
import 'audio_meters_feed.dart';
import 'audio_routes_frb.dart';
import 'keyframe_controls_frb.dart';
import 'mixer_keys.dart';
import 'mixer_master.dart';
import 'mixer_rack.dart';
import 'mixer_strip.dart';
import 'placeholder.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

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


/// The pan value as the well reads it: "C", "L 50", "R 30", a percentage of
/// the way to one side.
String panLabel(double pan) {
  final rounded = pan.round();
  if (rounded == 0) return l10n.panCentre;
  return rounded < 0 ? 'L ${-rounded}' : 'R $rounded';
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
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);

  /// Which layers can make a sound, probed once per layer and remembered,
  /// asked exactly as the Timeline asks it.
  final Map<String, bool> _hasAudio = {};

  /// The layers the panel was opened on, and those added since. A strip for
  /// one added since comes up into its place; the desk the panel opened on is
  /// simply there.
  Set<String>? _opening;
  final Set<String> _added = {};

  /// The strips as last laid out, left to right, and how wide each stood, for
  /// the keys and for Show bus.
  List<BridgeLayerEntry> _desk = const [];
  double _width = _stripWidth;

  /// The strips' own scroll, for the bar under them and the wheel.
  final ScrollController _strips = ScrollController();

  @override
  void initState() {
    super.initState();
    if (widget.feed == null) {
      meterPeakHoldSeconds = _ui.workspace.interface.peakHoldSeconds;
      _feed.start(
          wake: Listenable.merge([_ui.playing, _ui.playheadFrame]),
          playing: _ui.playing);
    }
    _feed.readPass = () {
      try {
        return _ui.selectedComp?.automationPass() ?? const [];
      } catch (_) {
        return const [];
      }
    };
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    if (widget.feed == null) _feed.dispose();
    _strips.dispose();
    super.dispose();
  }

  void _changed() {
    _ui.model.refresh();
    _ui.selectedComp?.audioPrepare();
  }

  /// A plain wheel turn carries the strips sideways: the list only runs one
  /// way, so there is nothing else for it to mean.
  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent || !_strips.hasClients) return;
    final by =
        event.scrollDelta.dx != 0 ? event.scrollDelta.dx : event.scrollDelta.dy;
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

  /// Bring [id]'s strip into view and pick it.
  void _show(UuidValue id) {
    final at = _desk.indexWhere((e) => e.layer.internallayerId == id);
    if (at < 0) return;
    _ui.setSelection([_desk[at].layer]);
    if (!_strips.hasClients) return;
    final p = _strips.position;
    final left = at * _width;
    final right = left + _width + _busGap;
    if (left < p.pixels) {
      _strips.jumpTo(left.clamp(0.0, p.maxScrollExtent));
    } else if (right > p.pixels + p.viewportDimension) {
      _strips.jumpTo(
          (right - p.viewportDimension).clamp(0.0, p.maxScrollExtent));
    }
  }

  // ---------------------------------------------------------------- the keys

  /// The Mixer's keys, answered while it is the focused panel. A dialogue, or
  /// a field being typed into, keeps its keys.
  bool _onKey(KeyEvent event) {
    if (event is KeyUpEvent || !mounted || lumitModalOpen) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    final ui = _ui;
    final comp = ui.selectedComp;
    if (ui.activePanel != Panel.mixer || comp == null || _desk.isEmpty) {
      return false;
    }
    final action = ui.keymap.actionFor(BridgeKeyContext.mixer, event);
    if (action == null) return false;
    // Only the arrows that nudge repeat while held.
    if (event is KeyRepeatEvent && !action.startsWith('mixer.fader.')) {
      return false;
    }
    if (!runMixerAction(
        action: action,
        ui: ui,
        app: Provider.of<LumitState>(context, listen: false),
        comp: comp,
        desk: _desk,
        positions: _feed.positions.value,
        show: _show)) {
      return false;
    }
    _changed();
    return true;
  }

  // -------------------------------------------------------------- the ground

  /// The desk's own ground, off every strip: new strips, and what can be said
  /// of all of them at once.
  void _groundMenu(CompositionReference comp, Offset at) {
    final ui = _ui;
    final desk = _desk;
    final app = Provider.of<LumitState>(context, listen: false);
    final muted = [
      for (final e in desk)
        if (!e.info.switches.audible) e.layer.internallayerId,
    ];
    final soloed = [
      for (final e in desk)
        if (e.info.switches.solo) e.layer.internallayerId,
    ];
    final raised = [
      for (final e in desk)
        if (levelMoved(e.info.volumeDb, comp, ui.playheadFrame.value)) e,
    ];
    final modes = {for (final e in desk) e.info.automationMode};

    void flip(BridgeLayerSwitch which, List<UuidValue> layers, bool on) {
      try {
        comp.setSwitchOnLayers(
            clicked: layers.first, layers: layers, switch_: which, on_: on);
      } catch (_) {}
      _changed();
    }

    showMenuAt<void>(
      context: context,
      position: at,
      width: 210,
      rows: (close) => [
        ..._insertRows(comp, () => close(null)),
        if (desk.isNotEmpty) ...[
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
              eachAsOne(
                  app,
                  raised,
                  (e) => e.layer.setVolumeDb(
                      value: scalarWithValueAt(
                          e.info.volumeDb, 0, comp, ui.playheadFrame.value)));
              _changed();
            },
            child: MenuLabel(l10n.mixerResetFaders, enabled: raised.isNotEmpty),
          ),
          SubmenuRow(
            key: const ValueKey<String>('mixer-automation'),
            closeParent: () => close(null),
            submenu: (dismiss) => FloatSurface(
              width: 130,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: automationRows(
                  modes.length == 1 ? modes.single : null,
                  (mode) {
                    dismiss();
                    eachAsOne(app, desk,
                        (e) => e.layer.setAutomationMode(mode: mode));
                    _changed();
                  },
                ),
              ),
            ),
            child: MenuLabel(l10n.audioAutomation),
          ),
        ],
      ],
    );
  }

  /// Insert audio layer and Insert bus, on the ground menu and on an empty
  /// desk.
  List<Widget> _insertRows(CompositionReference comp, VoidCallback close,
          {bool chords = true}) =>
      [
        for (final (action, label) in [
          ('layer.new.audio', l10n.insertAudioLayer),
          ('layer.new.bus', l10n.insertBus),
        ])
          MenuRow(
            key: ValueKey<String>('mixer-$action'),
            onPressed: () {
              close();
              insertEmptyLayer(_ui, comp, action);
              _changed();
            },
            child: MenuLabel(label, action: chords ? action : null),
          ),
      ];

  // ---------------------------------------------------------------- the desk

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
        final sounding = [
          for (final entry in layers)
            if (entry.info.kind != BridgeLayerKind.bus &&
                (_hasAudio[entry.layer.internallayerId.toString()] ?? false))
              entry,
        ];
        final buses = [
          for (final entry in layers)
            if (entry.info.kind == BridgeLayerKind.bus) entry,
        ];
        final desk = _desk = [...sounding, ...buses];
        return LayoutBuilder(
          builder: (context, box) {
            final height = math.max(box.maxHeight, _deskFloor);
            final loudness = _loudnessRoom(box.maxWidth, desk.length,
                sounding.isNotEmpty && buses.isNotEmpty);
            final row = Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Expanded(
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onSecondaryTapUp: (e) =>
                        _groundMenu(comp, e.globalPosition),
                    child: desk.isEmpty
                        ? _empty(t, comp)
                        : _stripList(context, comp, desk, sounding.length,
                            buses.length, height),
                  ),
                ),
                MasterStrip(
                  comp: comp,
                  feed: _feed,
                  width: _masterWidth + (loudness ? _loudnessWidth : 0),
                  rackHeight: _rack(desk, buses.isNotEmpty, height).rows *
                      rackRow,
                  loudness: loudness,
                  onChanged: _changed,
                ),
              ],
            );
            return Container(
              color: t.surface0,
              child: box.maxHeight >= _deskFloor
                  ? row
                  // Too short for a strip: it keeps its floor and is cut.
                  : ClipRect(
                      child: OverflowBox(
                        alignment: Alignment.topCenter,
                        minHeight: _deskFloor,
                        maxHeight: _deskFloor,
                        child: row,
                      ),
                    ),
            );
          },
        );
      },
    );
  }

  /// A comp with nothing to mix: the words, and the two ways to start.
  Widget _empty(LumitTheme t, CompositionReference comp) => Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 180),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(l10n.mixerNoSound,
                  style: t.small, textAlign: TextAlign.center),
              const SizedBox(height: 8),
              ..._insertRows(comp, () {}, chords: false),
            ],
          ),
        ),
      );

  /// The strips, layers then buses after a gap, scrolled sideways under a bar
  /// that says there are more than the panel shows.
  Widget _stripList(BuildContext context, CompositionReference comp,
      List<BridgeLayerEntry> desk, int layerCount, int busCount, double height) {
    final reveal = ThemeScope.of(context).motion;
    final t = ThemeScope.of(context).theme;
    final app = Provider.of<LumitState>(context, listen: false);
    final rack = _rack(desk, busCount > 0, height);
    final silenced = soloSilenced(desk);
    final gap = layerCount > 0 && busCount > 0 ? _busGap : 0.0;
    final scope = RackScope(
      ui: _ui,
      app: app,
      comp: comp,
      onChanged: _changed,
      showBus: _show,
      positions: _feed.positions,
    );
    Widget strip(BridgeLayerEntry entry, double width) {
      final id = entry.layer.internallayerId;
      return Entrance(
        key: ValueKey('strip-in-$id'),
        spec: _added.contains(id.toString()) ? reveal.reveal : MotionSpec.still,
        rise: reveal.revealRise,
        child: MixerStrip(
          key: ValueKey('strip-$id'),
          comp: comp,
          entry: entry,
          desk: desk,
          width: width,
          rack: rack,
          scope: scope,
          feed: _feed,
          silenced: silenced.contains(id),
        ),
      );
    }

    // The strips stop a gap short of the Master, so a strip cut at the edge
    // fades out before its scale meets the Master's.
    return Padding(
      padding: const EdgeInsets.only(right: _deskGap),
      child: Listener(
        onPointerSignal: _wheel,
        child: LayoutBuilder(
          builder: (context, box) {
            final width =
                _width = _stripWidthIn(box.maxWidth - gap, desk.length);
            return HouseScrollbar(
              controller: _strips,
              crossAxisMargin: 2,
              // Shown for as long as strips are out of sight.
              thumbVisibility: width * desk.length + gap > box.maxWidth + 0.5,
              child: EdgeFade(
                reach: _fadeReach,
                child: ListView(
                  controller: _strips,
                  scrollDirection: Axis.horizontal,
                  children: [
                    for (final entry in desk.take(layerCount))
                      strip(entry, width),
                    // The buses stand on their own ground, a hairline along.
                    if (gap > 0)
                      Container(
                        width: gap,
                        decoration: BoxDecoration(
                          border: Border(right: BorderSide(color: t.hairline)),
                        ),
                      ),
                    for (final entry in desk.skip(layerCount))
                      strip(entry, width),
                  ],
                ),
              ),
            );
          },
        ),
      ),
    );
  }

  /// How the racks stand: a row per effect and per send, plus each section's
  /// + row, while the panel is tall enough to leave the faders their throw,
  /// else one counting row each. Every strip takes the tallest, so the faders
  /// stand level.
  RackLayout _rack(List<BridgeLayerEntry> desk, bool sendable, double height) {
    var fx = 0, sends = 0;
    for (final e in desk) {
      fx = math.max(fx, inserts(e.info).length);
      sends = math.max(sends, e.info.sends.length);
    }
    final insertRows = fx + 1;
    final sendRows = sendable ? sends + 1 : 0;
    final room = height - _stripFixed - _faderFloor;
    if (height >= _tallDesk && (insertRows + sendRows) * rackRow <= room) {
      return RackLayout(
          expanded: true, insertRows: insertRows, sendRows: sendRows);
    }
    final short = height < _stripFixed + _faderFloor + 2 * rackRow;
    return RackLayout(
        expanded: false,
        insertRows: short ? 0 : 1,
        sendRows: short || !sendable ? 0 : 1);
  }

  /// Whether the Master's loudness can stand beside its meter without costing
  /// the desk a strip, on a panel [width] wide.
  bool _loudnessRoom(double width, int count, bool gap) {
    final room = width - _masterWidth - _deskGap - (gap ? _busGap : 0);
    return _wholeStrips(room - _loudnessWidth, count) ==
        _wholeStrips(room, count);
  }
}

extension on RackLayout {
  int get rows => insertRows + sendRows;
}

/// How wide a strip stands, and the most one takes when the desk has room.
const double _stripWidth = 58;
const double _stripWidthMost = 84;

/// How wide the Master stands, and how much wider to hold its loudness.
const double _masterWidth = 70;
const double _loudnessWidth = 46;

/// The desk's own ground kept beside strips that all fit, for its right-click.
const double _deskGround = 16;

/// The gap before the buses.
const double _busGap = 6;

/// The gap between the strips and the Master, and how far in from a cut edge
/// the strips fade: most of a part strip, so its numbers never read. The gap
/// is the desk's own ground however many strips there are, so its right-click
/// is always in reach.
const double _deskGap = 14;
const double _fadeReach = 34;

/// Everything on a strip but its rack and its fader, top to bottom.
const double _stripFixed = 12 +
    stripNameRow +
    stripPotRow +
    stripChipRow +
    16 +
    stripSwitchRow +
    stripChipRow +
    stripGap * 7;

/// The least throw a fader keeps before the racks fold to a count, the height
/// under which they always do, and the height a strip is never drawn under.
const double _faderFloor = 120;
const double _tallDesk = 420;
const double _deskFloor = 200;

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
