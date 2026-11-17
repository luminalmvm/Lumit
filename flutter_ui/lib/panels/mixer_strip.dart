// One Mixer strip, a layer's or a bus's: the name, the Gain and Pan pots, the
// rack, the automation mode, the fader beside its meters, the dB well, the
// switches and where its sound goes. Every part answers a right-click, and
// every strip's sections stand the same height, so the faders stay level.

import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/dialog_frame.dart' show FittedText;
import '../shell/menu_bar_frb.dart' show insertEmptyLayer;
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'audio_meters_feed.dart';
import 'audio_routes_frb.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'mixer_keys.dart';
import 'mixer_live.dart';
import 'mixer_meter.dart';
import 'mixer_panel_frb.dart' show meterRows, mixValueRows, panLabel;
import 'mixer_rack.dart';
import 'timeline_extras_frb.dart'
    show iconForKind, rowSelectionFill, showMenuAt;
import 'timeline_outline_row_frb.dart' show RowRenameField;

/// How tall the rows that must stand level across the desk are.
const double stripNameRow = 18;
const double stripPotRow = 28;
const double stripChipRow = 14;
const double stripSwitchRow = 14;
const double stripGap = 4;

/// The fader, the gap and the meter band side by side, which every strip
/// centres; the clip lamp stands over the meter's bars.
const double _faderRowWidth = 26 + 3 + 14;
const double _lampInset = 3;

/// How the desk has its strips' racks stand: rows each, or a count.
class RackLayout {
  final bool expanded;
  final int insertRows;

  /// Nought where the comp has no bus to send to.
  final int sendRows;

  const RackLayout(
      {required this.expanded,
      required this.insertRows,
      required this.sendRows});
}

/// An automation mode's colour: Read is quiet unless the strip has keys to
/// read, Touch, Latch and Trim warn, Write is the error colour.
Color automationInk(LumitTheme t, BridgeAutomationMode mode, bool keyed) =>
    switch (mode) {
      BridgeAutomationMode.read => keyed ? t.accent : t.textMuted,
      BridgeAutomationMode.write => t.error,
      _ => t.warning,
    };

/// The automation modes as menu rows, [current] ticked.
List<Widget> automationRows(BridgeAutomationMode? current,
        void Function(BridgeAutomationMode mode) pick,
        {List<BridgeAutomationMode> modes = BridgeAutomationMode.values}) =>
    [
      for (final mode in modes)
        MenuRow(
          key: ValueKey<String>('mixer-mode-${mode.name}'),
          onPressed: () => pick(mode),
          child: MenuLabel(automationModeLabel(mode), ticked: current == mode),
        ),
    ];

class MixerStrip extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

  /// Every strip on the desk, which a menu on a picked strip acts across.
  final List<BridgeLayerEntry> desk;
  final double width;
  final RackLayout rack;
  final RackScope scope;
  final AudioMeterFeed feed;

  /// Whether another strip's solo silences this one, which dims its body.
  final bool silenced;

  const MixerStrip({
    super.key,
    required this.comp,
    required this.entry,
    required this.desk,
    required this.width,
    required this.rack,
    required this.scope,
    required this.feed,
    required this.silenced,
  });

  @override
  State<MixerStrip> createState() => _MixerStripState();
}

class _MixerStripState extends State<MixerStrip> {
  /// Mid-gesture values, shown live and written once on release, so the whole
  /// gesture is one undo step.
  double? _dragDb, _dragPan, _dragGain;

  /// Where the fader and the pot stood when they were taken hold of, for
  /// Escape to put them back.
  double _fromDb = 0, _fromPan = 0;

  /// The name's field while the strip is being renamed.
  TextEditingController? _rename;

  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);

  BridgeLayerEntry get _entry => widget.entry;
  CompositionReference get _comp => widget.comp;
  int get _frame => _ui.playheadFrame.value;

  @override
  void dispose() {
    _rename?.dispose();
    super.dispose();
  }

  void _changed() => widget.scope.onChanged();

  // ------------------------------------------------------------ the gestures

  void _volumeLive(double db) {
    if (_dragDb == null) _fromDb = _shownDb(_frame);
    setState(() => _dragDb = db);
    mixerTouch(_comp, volumeOf(_entry), db);
  }

  void _volumeCommit(double db) {
    mixerTouch(_comp, volumeOf(_entry), db);
    mixerRelease(_comp, volumeOf(_entry), _ui);
    setState(() => _dragDb = null);
    _changed();
  }

  void _volumeCancel() {
    mixerCancel(_comp, volumeOf(_entry), _fromDb, _ui);
    setState(() => _dragDb = null);
  }

  void _panLive(double v) {
    if (_dragPan == null) _fromPan = _shownPan(_frame);
    setState(() => _dragPan = v);
    mixerTouch(_comp, panOf(_entry), v);
  }

  void _panCommit(double v) {
    mixerTouch(_comp, panOf(_entry), v);
    mixerRelease(_comp, panOf(_entry), _ui);
    setState(() => _dragPan = null);
    _changed();
  }

  void _panCancel() {
    mixerCancel(_comp, panOf(_entry), _fromPan, _ui);
    setState(() => _dragPan = null);
  }

  void _gainCommit(double db) {
    setState(() => _dragGain = null);
    if (db == _entry.info.audioGainDb) return;
    try {
      _entry.layer.setAudioGainDb(db: db);
    } catch (_) {
      return;
    }
    _changed();
  }

  void _soloTap() {
    final keys = HardwareKeyboard.instance;
    final info = _entry.info;
    try {
      if (keys.isControlPressed) {
        _entry.layer.setSoloSafe(enabled: !info.soloSafe);
      } else if (keys.isAltPressed) {
        soloAlone(widget.scope.app, _comp, widget.desk, [_entry]);
      } else {
        _entry.layer.setSwitch(
            switch_: BridgeLayerSwitch.solo, on_: !info.switches.solo);
      }
    } catch (_) {
      return;
    }
    _changed();
  }

  void _beginRename() {
    if (_entry.info.switches.locked) return;
    setState(() => _rename = TextEditingController(text: _entry.info.name));
  }

  void _endRename({required bool commit}) {
    final text = _rename?.text.trim();
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    if (commit && text != null && text.isNotEmpty && text != _entry.info.name) {
      _entry.layer.rename(name: text);
      _changed();
    }
  }

  // ------------------------------------------------------------- the menus

  List<BridgeLayerEntry> get _targets =>
      audioMenuTargets(_entry, widget.desk, _ui.selectedLayerIds);

  /// The strip's own right-click: the same rows a track head has in the Audio
  /// timeline, since it is the same layer.
  void _stripMenu(Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 200,
      rows: (close) => audioLayerRows(
        theme: ThemeScope.of(context).theme,
        comp: _comp,
        app: widget.scope.app,
        entry: _entry,
        targets: _targets,
        buses: widget.scope.buses,
        frame: _frame,
        closeParent: () => close(null),
        onRename: _beginRename,
        onChanged: _changed,
        renameAction: null,
        duplicateAction: null,
        normaliseTargetDb: _ui.workspace.normaliseTargetDb,
      ),
    );
  }

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
    showMenuAt<void>(
      context: context,
      position: at,
      width: 190,
      rows: (close) => mixValueRows(
        context: context,
        comp: _comp,
        frame: _frame,
        scalar: scalar,
        write: write,
        onChanged: _changed,
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
        scalar: _entry.info.volumeDb,
        write: (next) => _entry.layer.setVolumeDb(value: next),
        resetLabel: l10n.audioResetVolume,
        askLabel: l10n.mixerSetLevel,
        title: l10n.volume,
        min: faderFloorDb,
        max: 12,
        decimals: 1,
        suffix: ' ${l10n.unitSymbolDb}',
      );

  void _potMenu(Offset at) => _valueMenu(
        at,
        scalar: _entry.info.pan,
        write: (next) => _entry.layer.setPan(value: next),
        resetLabel: l10n.audioResetPan,
        askLabel: l10n.mixerSetPan,
        title: l10n.audioPan,
        min: -100,
        max: 100,
        decimals: 0,
        sides: [(l10n.mixerPanLeft, -100), (l10n.mixerPanRight, 100)],
      );

  void _meterMenu(Offset at) => showMenuAt<void>(
        context: context,
        position: at,
        width: 170,
        rows: (close) => meterRows(_ui, () => close(null)),
      );

  void _modeMenu(BuildContext chip) {
    final box = chip.findRenderObject();
    if (box is! RenderBox) return;
    final targets = _targets;
    showMenuAt<void>(
      context: chip,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      width: 130,
      rows: (close) => automationRows(_entry.info.automationMode, (mode) {
        close(null);
        eachAsOne(widget.scope.app, targets,
            (t) => t.layer.setAutomationMode(mode: mode));
        _changed();
      }),
    );
  }

  /// Output: the master, a bus, or a new bus this strip goes to.
  void _outputMenu(BuildContext chip) {
    final box = chip.findRenderObject();
    if (box is! RenderBox) return;
    final self = _entry.layer.internallayerId;
    final output = _entry.info.output;
    void route(UuidValue? to) {
      try {
        _entry.layer.setAudioRoutes(output: to, sends: _entry.info.sends);
      } catch (_) {
        widget.scope.app.postNotice(l10n.audioRouteRefused);
        return;
      }
      _changed();
    }

    showMenuAt<void>(
      context: chip,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      width: 170,
      rows: (close) => [
        MenuRow(
          key: const ValueKey<String>('output-master'),
          onPressed: () {
            close(null);
            if (output != null) route(null);
          },
          child: MenuLabel(l10n.audioRouteMaster, ticked: output == null),
        ),
        for (final bus in widget.scope.buses)
          if (bus.layer.internallayerId != self)
            () {
              final id = bus.layer.internallayerId;
              final on = output == id;
              final allowed = on || _comp.routeAllowed(layer: self, bus: id);
              return MenuRow(
                key: ValueKey<String>('output-$id'),
                enabled: allowed,
                onPressed: () {
                  close(null);
                  if (!on) route(id);
                },
                child: MenuLabel(bus.info.name, ticked: on, enabled: allowed),
              );
            }(),
        const MenuDivider(),
        MenuRow(
          key: const ValueKey<String>('output-new-bus'),
          onPressed: () {
            close(null);
            asOneUndoStep(widget.scope.app.project, () {
              final bus = insertEmptyLayer(_ui, _comp, 'layer.new.bus');
              if (bus == null) return;
              try {
                _entry.layer.setAudioRoutes(
                    output: bus.internallayerId, sends: _entry.info.sends);
              } catch (_) {
                // A locked layer keeps its route; the bus is still made.
              }
            });
            _changed();
          },
          child: MenuLabel(l10n.mixerNewBus, ticked: false),
        ),
      ],
    );
  }

  // ------------------------------------------------------------ the drawing

  double _sampled(BridgeScalar scalar, int frame) =>
      scalar is BridgeScalar_Static
          ? scalar.field0
          : sampledScalar(scalar, timeOfFrame(_comp, frame));

  StripPosition? get _placed =>
      widget.feed.positions.value[_entry.layer.internallayerId.toString()];

  /// The fader as it stands: held, then where the automation has it while
  /// playing, then the model at the playhead.
  double _shownDb(int frame) =>
      _dragDb ?? _placed?.volumeDb ?? _sampled(_entry.info.volumeDb, frame);

  double _shownPan(int frame) =>
      _dragPan ?? _placed?.pan ?? _sampled(_entry.info.pan, frame);

  /// Whether an automation pass is writing this strip's fader now.
  bool get _recording =>
      widget.feed.recording.value.contains(volumeOf(_entry));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final info = _entry.info;
    final layer = _entry.layer;
    final ui = _ui;
    final mark = ThemeScope.of(context).motion.mark;
    // Keyed Volume, Pan or a send reads at the playhead, and only then does
    // the strip listen to it.
    final animated = info.volumeDb is! BridgeScalar_Static ||
        info.pan is! BridgeScalar_Static;

    // A press anywhere on the strip picks its layer, on the way down so the
    // pick lands before the fader or the pot starts its own gesture.
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
            builder: (context, picked, child) {
              final ground =
                  info.kind == BridgeLayerKind.bus ? t.surface1 : t.surface0;
              final on =
                  picked.any((l) => l.internallayerId == layer.internallayerId);
              return AnimatedContainer(
                duration: mark.duration,
                curve: mark.curve,
                decoration: BoxDecoration(
                  color: on
                      ? Color.alphaBlend(rowSelectionFill(t), ground)
                      : ground,
                  border: Border(right: BorderSide(color: t.hairline)),
                ),
                padding: const EdgeInsets.symmetric(vertical: 6),
                child: child,
              );
            },
            child: !animated
                ? _body(context, t, info)
                : ValueListenableBuilder<int>(
                    valueListenable: ui.playheadFrame,
                    builder: (context, _, __) => _body(context, t, info),
                  ),
          ),
        ),
      ),
    );
  }

  /// Dims what the solo silences: everything but the name and the switches.
  Widget _dim(Widget child) => AnimatedOpacity(
        opacity: widget.silenced ? 0.45 : 1,
        duration: ThemeScope.of(context).motion.mark.duration,
        child: child,
      );

  /// Rebuilt as the faders move with the automation, and as a pass starts
  /// and stops writing them.
  Widget _following(Widget Function() build) => ListenableBuilder(
        listenable:
            Listenable.merge([widget.feed.positions, widget.feed.recording]),
        builder: (context, _) => build(),
      );

  Widget _body(BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    final layer = _entry.layer;
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    final isBus = info.kind == BridgeLayerKind.bus;
    final chain = info.effects.where((e) => e.enabled && e.audio).length;
    final keyed = info.volumeDb is BridgeScalar_Keyframed ||
        info.pan is BridgeScalar_Keyframed ||
        info.sends.any((s) => s.level is BridgeScalar_Keyframed);
    final rack = widget.rack;
    String? bus;
    for (final b in widget.scope.buses) {
      if (b.layer.internallayerId == info.output) bus = b.info.name;
    }
    final gain = _dragGain ?? info.audioGainDb;
    return Column(
      children: [
        _name(t, info),
        const SizedBox(height: stripGap),
        _dim(SizedBox(
          height: stripPotRow,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // A bus has no Gain of its own, and its Pan stands where every
              // strip's does.
              _pot(
                t,
                isBus ? null : l10n.audioGain,
                _dragGain == null ? null : _signedDb(gain),
                isBus
                    ? null
                    : PanPot(
                        key: ValueKey('gain-$id'),
                        value: gain,
                        range: 24,
                        tip: l10n.mixerGainTip(_signedDb(gain)),
                        onLive: (v) => setState(() => _dragGain = v),
                        onCommit: _gainCommit,
                        onCancel: () => setState(() => _dragGain = null),
                      ),
              ),
              _pot(
                t,
                l10n.audioPan,
                _dragPan == null ? null : panLabel(_dragPan!),
                GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onSecondaryTapUp: (e) => _potMenu(e.globalPosition),
                  child: _following(() => PanPot(
                        key: ValueKey('pot-$id'),
                        value: _shownPan(_frame),
                        onLive: _panLive,
                        onCommit: _panCommit,
                        onCancel: _panCancel,
                      )),
                ),
              ),
            ],
          ),
        )),
        const SizedBox(height: stripGap),
        _dim(RackSection(
          scope: widget.scope,
          entry: _entry,
          sends: false,
          expanded: rack.expanded,
          rows: rack.insertRows,
          width: widget.width,
        )),
        if (rack.sendRows > 0)
          _dim(RackSection(
            scope: widget.scope,
            entry: _entry,
            sends: true,
            expanded: rack.expanded,
            rows: rack.sendRows,
            width: widget.width,
          )),
        const SizedBox(height: stripGap),
        // The mode sits over the scale and the clip lamp over the meter, so
        // the two share a row.
        _dim(SizedBox(
          height: stripChipRow,
          child: Row(
            children: [
              const SizedBox(width: 2),
              Expanded(
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: MixChip(
                    key: ValueKey('mode-$id'),
                    label: automationModeLabel(info.automationMode),
                    ink: automationInk(t, info.automationMode, keyed),
                    quiet: info.automationMode == BridgeAutomationMode.read,
                    tip: l10n.audioAutomation,
                    onTap: _modeMenu,
                  ),
                ),
              ),
              const SizedBox(width: 2),
              ClipLamp(stripId: id, feed: widget.feed),
              SizedBox(
                  width: (widget.width - _faderRowWidth) / 2 + _lampInset),
            ],
          ),
        )),
        const SizedBox(height: stripGap),
        Expanded(
          child: _dim(Row(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              GestureDetector(
                onSecondaryTapUp: (e) => _faderMenu(e.globalPosition),
                child: _following(() => MixFader(
                      key: ValueKey('fader-$id'),
                      db: _shownDb(_frame),
                      recording: _recording,
                      onLive: _volumeLive,
                      onCommit: _volumeCommit,
                      onCancel: _volumeCancel,
                    )),
              ),
              const SizedBox(width: 3),
              GestureDetector(
                behavior: HitTestBehavior.opaque,
                onSecondaryTapUp: (e) => _meterMenu(e.globalPosition),
                child: MeterBand(stripId: id, feed: widget.feed),
              ),
            ],
          )),
        ),
        const SizedBox(height: stripGap),
        _dim(_following(
            () => DbWell(db: _shownDb(_frame), onCommit: _volumeCommit))),
        const SizedBox(height: stripGap),
        SizedBox(
          height: stripSwitchRow,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              SwitchGlyph(
                keyName: 'mute-$id',
                mark: switches.audible ? LumitIcons.audio : LumitIcons.muted,
                on: switches.audible,
                tip: switches.audible ? l10n.switchAudible : l10n.switchMuted,
                onTap: () {
                  layer.setSwitch(
                      switch_: BridgeLayerSwitch.audible,
                      on_: !switches.audible);
                  _changed();
                },
              ),
              const SizedBox(width: 5),
              SoloChip(
                keyName: 'solo-$id',
                on: switches.solo,
                safe: info.soloSafe,
                onTap: _soloTap,
              ),
              // A strip with plugins in its sound can bypass them from here.
              if (chain > 0) ...[
                const SizedBox(width: 5),
                SwitchGlyph(
                  keyName: 'chain-$id',
                  icon: LumitIcon.fx,
                  on: switches.fx,
                  tip: l10n.mixerChain(chain),
                  onTap: () {
                    layer.setSwitch(
                        switch_: BridgeLayerSwitch.fx, on_: !switches.fx);
                    _changed();
                  },
                ),
              ],
            ],
          ),
        ),
        const SizedBox(height: stripGap),
        _dim(SizedBox(
          height: stripChipRow,
          child: MixChip(
            key: ValueKey('output-$id'),
            label: bus ?? l10n.audioRouteMaster,
            ink: bus == null ? t.textMuted : t.textSecondary,
            width: widget.width - 8,
            onTap: _outputMenu,
          ),
        )),
      ],
    );
  }

  /// A gain in dB as its caption reads while it turns: signed, to a tenth.
  static String _signedDb(double db) =>
      '${db > 0 ? '+' : ''}${db.toStringAsFixed(1)}';

  /// One pot over its caption, which reads the value while it turns. A null
  /// [pot] keeps the place empty, as a bus's Gain.
  Widget _pot(LumitTheme t, String? caption, String? turning, Widget? pot) =>
      SizedBox(
        width: 26,
        child: Column(
          children: [
            SizedBox(height: 14, child: pot),
            const SizedBox(height: 1),
            Text(
              turning ?? caption ?? '',
              maxLines: 1,
              softWrap: false,
              overflow: TextOverflow.visible,
              style: t.caption.copyWith(
                  color: turning == null ? t.textMuted : t.textPrimary),
            ),
          ],
        ),
      );

  /// The strip's name, with its label colour under it and the bus glyph
  /// before a bus's, or the field it turns into. A double-click renames.
  Widget _name(LumitTheme t, BridgeLayerInfo info) => SizedBox(
        height: stripNameRow,
        child: _rename != null
            ? RowRenameField(
                key: const ValueKey<String>('strip-rename'),
                controller: _rename!,
                onCommit: () => _endRename(commit: true),
                onCancel: () => _endRename(commit: false),
              )
            : GestureDetector(
                key: ValueKey<String>(
                    'strip-name-${_entry.layer.internallayerId}'),
                behavior: HitTestBehavior.opaque,
                onDoubleTap: _beginRename,
                child: Container(
                  constraints: BoxConstraints(maxWidth: widget.width - 8),
                  decoration: BoxDecoration(
                    border: Border(
                      bottom: BorderSide(
                          color: t.labelColour(info.label), width: 2),
                    ),
                  ),
                  padding: const EdgeInsets.only(bottom: 2),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      if (info.kind == BridgeLayerKind.bus) ...[
                        lumitIcon(iconForKind(BridgeLayerKind.bus),
                            size: 10, color: t.textMuted),
                        const SizedBox(width: 3),
                      ],
                      // A name cut short says the whole of itself on hover.
                      Flexible(
                        child: FittedText(
                            info.name, t.small.copyWith(color: t.textPrimary)),
                      ),
                    ],
                  ),
                ),
              ),
      );
}
