// A Mixer strip's rack: its audio effects (the inserts) and its sends, one
// short row each, or one row counting them where the panel is short, which
// opens the rows in a popover.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart' show BridgeMixTarget;
import 'package:lumit_flutter/src/rust/api/routes.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/comp_time.dart';
import '../state/dock.dart' show Panel, setPanelVisible;
import '../state/ui_state.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import 'audio_routes_frb.dart' show writeSend;
import 'effect_controls_panel_frb.dart' show showAddEffectMenu;
import 'effect_param_row_frb.dart' show effectLabelOf;
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'mixer_live.dart';
import 'mixer_meter.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

/// How tall one rack row stands.
const double rackRow = 16;

/// What a rack section needs from its strip.
class RackScope {
  final LumitUiState ui;
  final LumitState app;
  final CompositionReference comp;
  final VoidCallback onChanged;

  /// Where a send's bus strip is brought into view, by bus id.
  final void Function(UuidValue bus) showBus;

  /// Where the controls stand while the comp plays, from the meter feed.
  final ValueListenable<Map<String, StripPosition>> positions;

  const RackScope({
    required this.ui,
    required this.app,
    required this.comp,
    required this.onChanged,
    required this.showBus,
    required this.positions,
  });

  /// [id]'s entry as the model now holds it, so a popover left open follows
  /// the edits made in it.
  BridgeLayerEntry? entryOf(UuidValue id) {
    for (final e in ui.model.heldLayers) {
      if (e.layer.internallayerId == id) return e;
    }
    return null;
  }

  /// The comp's buses, in layer order.
  List<BridgeLayerEntry> get buses => [
        for (final e in ui.model.heldLayers)
          if (e.info.kind == BridgeLayerKind.bus) e,
      ];
}

/// The audio effects a strip plays through.
List<BridgeEffectInstanceInfo> inserts(BridgeLayerInfo info) => [
      for (final e in info.effects)
        if (e.audio) e
    ];

/// A rack section: its rows when [expanded], else one row counting them that
/// opens the rows in a popover.
class RackSection extends StatelessWidget {
  final RackScope scope;
  final BridgeLayerEntry entry;
  final bool sends;
  final bool expanded;

  /// How many rows the section stands, so every strip's fader starts level.
  final int rows;
  final double width;

  const RackSection({
    super.key,
    required this.scope,
    required this.entry,
    required this.sends,
    required this.expanded,
    required this.rows,
    required this.width,
  });

  Widget _rows(BridgeLayerEntry e) => sends
      ? SendRows(scope: scope, entry: e, width: width)
      : InsertRows(scope: scope, entry: e, width: width);

  @override
  Widget build(BuildContext context) {
    if (rows == 0) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    final id = entry.layer.internallayerId;
    final count = sends ? entry.info.sends.length : inserts(entry.info).length;
    return SizedBox(
      height: rows * rackRow,
      child: expanded
          ? Align(alignment: Alignment.topCenter, child: _rows(entry))
          : _RackRow(
              key: ValueKey<String>('${sends ? 'sends' : 'fx'}-count-$id'),
              empty: count == 0,
              onTap: (row) {
                final box = row.findRenderObject();
                if (box is! RenderBox) return;
                showLumitPopup<void>(
                  context: row,
                  position: box.localToGlobal(Offset(0, box.size.height)),
                  builder: (close) => FloatSurface(
                    width: width + 40,
                    child: ListenableBuilder(
                      listenable: scope.ui.model,
                      builder: (context, _) {
                        final now = scope.entryOf(id);
                        return now == null ? const SizedBox() : _rows(now);
                      },
                    ),
                  ),
                );
              },
              child: Text(
                sends
                    ? l10n.mixerSendsCount(count)
                    : l10n.mixerInsertsCount(count),
                maxLines: 1,
                style: t.caption.copyWith(
                    color: count == 0 ? t.textMuted : t.textSecondary),
              ),
            ),
    );
  }
}

/// One rack slot, the strip's width less a margin: filled when it holds an
/// effect or a send, an empty outline when it adds one. A hover lifts it, a
/// click and a right-click answer.
class _RackRow extends StatelessWidget {
  final Widget? child;
  final void Function(BuildContext row)? onTap;
  final void Function(Offset at)? onMenu;
  final String? tip;

  /// Drawn as an empty slot although it has words, as a count of none.
  final bool empty;

  const _RackRow(
      {super.key,
      this.child,
      this.onTap,
      this.onMenu,
      this.tip,
      this.empty = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final radius = BorderRadius.circular(t.tokens.controlRadius);
    final row = HoverMark(
      cursor: onTap == null ? MouseCursor.defer : SystemMouseCursors.click,
      builder: (context, lit) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap == null ? null : () => onTap!(context),
        onSecondaryTapUp:
            onMenu == null ? null : (e) => onMenu!(e.globalPosition),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
          child: Container(
            height: rackRow - 2,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            alignment: Alignment.centerLeft,
            decoration: child == null || empty
                // An empty slot: an outline, filled faintly under the pointer.
                ? BoxDecoration(
                    // Sunk into the ground, so it reads on a picked strip.
                    color: Color.lerp(t.surface0, t.surface3, 0.5 * lit),
                    // Strong enough to stand on a picked strip's fill too.
                    border: Border.all(
                        color: t.hairlineStrong
                            .withValues(alpha: 0.55 + 0.45 * lit)),
                    borderRadius: radius,
                  )
                : BoxDecoration(
                    color: Color.lerp(t.surface2, t.surface3, lit),
                    borderRadius: radius,
                  ),
            child: child ??
                Center(
                  child: Text('+',
                      style: t.caption.copyWith(
                          color: Color.lerp(
                              t.textDisabled, t.textSecondary, lit))),
                ),
          ),
        ),
      ),
    );
    return tip == null ? row : LumitTooltip(message: tip!, child: row);
  }
}

/// The empty slot closing a section, which adds to it.
Widget _addRow(String key, String tip, void Function(BuildContext row) onTap) =>
    _RackRow(key: ValueKey<String>(key), tip: tip, onTap: onTap);

/// The strip's audio effects, one row each, and a + row that adds one.
class InsertRows extends StatelessWidget {
  final RackScope scope;
  final BridgeLayerEntry entry;
  final double width;

  const InsertRows(
      {super.key,
      required this.scope,
      required this.entry,
      required this.width});

  /// The live handle for [id], read at the moment of the gesture.
  BridgeEffectInstance? _instance(UuidValue id) {
    for (final e in entry.layer.getEffects()) {
      if (e.id() == id) return e;
    }
    return null;
  }

  void _edit(void Function() write) {
    try {
      write();
    } catch (_) {
      // A locked layer refuses, and quietly.
      return;
    }
    scope.onChanged();
  }

  /// Pick the effect and bring Effect controls to the front on it.
  void _front(BridgeEffectInstanceInfo effect) {
    final ui = scope.ui;
    ui.setSelection([entry.layer]);
    ui.setEffectSelection(entry.layer, [effect.id]);
    setPanelVisible(ui.split, Panel.effectControls, true);
    ui.frontPanel(Panel.effectControls);
  }

  void _menu(BuildContext context, Offset at, BridgeEffectInstanceInfo effect,
      List<BridgeEffectInstanceInfo> rack) {
    final all = entry.info.effects;
    final i = rack.indexOf(effect);
    // Up and down among the audio effects, wherever the stack has others.
    int? to(int step) {
      final j = i + step;
      if (j < 0 || j >= rack.length) return null;
      return all.indexWhere((e) => e.id == rack[j].id);
    }

    final up = to(-1);
    final down = to(1);
    Widget row(String key, String label, VoidCallback? act, {bool? ticked}) =>
        MenuRow(
          key: ValueKey<String>('insert-menu-$key'),
          enabled: act != null,
          onPressed: act ?? () {},
          child:
              MenuLabel(label, ticked: ticked ?? false, enabled: act != null),
        );
    showMenuAt<void>(
      context: context,
      position: at,
      width: 160,
      rows: (close) {
        void run(void Function(BridgeEffectInstance e) write) {
          close(null);
          final e = _instance(effect.id);
          if (e != null) _edit(() => write(e));
        }

        return [
          row(
              'on',
              l10n.mixerInsertOn,
              () => run((e) => entry.layer
                  .setEffectEnabled(effect: e, enabled: !effect.enabled)),
              ticked: effect.enabled),
          row('remove', l10n.mixerInsertRemove,
              () => run((e) => entry.layer.removeEffect(effect: e))),
          const MenuDivider(),
          row(
              'up',
              l10n.moveUp,
              up == null
                  ? null
                  : () => run((e) =>
                      entry.layer.reorderEffect(effect: e, newIndex: up))),
          row(
              'down',
              l10n.moveDown,
              down == null
                  ? null
                  : () => run((e) =>
                      entry.layer.reorderEffect(effect: e, newIndex: down))),
        ];
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final rack = inserts(entry.info);
    final id = entry.layer.internallayerId;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final effect in rack)
          _RackRow(
            key: ValueKey<String>('insert-${effect.id}'),
            tip: effect.customName ?? effectLabelOf(effect.name),
            onTap: (_) => _front(effect),
            onMenu: (at) => _menu(context, at, effect, rack),
            child: Text(
              effect.customName ?? effectLabelOf(effect.name),
              maxLines: 1,
              softWrap: false,
              overflow: TextOverflow.ellipsis,
              style: t.caption.copyWith(
                  color: effect.enabled ? t.textSecondary : t.textDisabled),
            ),
          ),
        _addRow(
            'insert-add-$id',
            l10n.audioAddEffect,
            (row) => showAddEffectMenu(row, (name) {
                  _edit(() => entry.layer.addEffect(name: name));
                }, category: 'audio')),
      ],
    );
  }
}

/// The strip's sends, one row each with its level bar and Pre chip, and a +
/// row listing the buses it may send to.
class SendRows extends StatelessWidget {
  final RackScope scope;
  final BridgeLayerEntry entry;
  final double width;

  const SendRows(
      {super.key,
      required this.scope,
      required this.entry,
      required this.width});

  void _routes(List<BridgeSend> sends) {
    try {
      entry.layer.setAudioRoutes(output: entry.info.output, sends: sends);
    } catch (_) {
      scope.app.postNotice(l10n.audioRouteRefused);
      return;
    }
    scope.onChanged();
  }

  void _menu(BuildContext context, Offset at, BridgeSend send) {
    Widget row(String key, String label, VoidCallback act, {bool? ticked}) =>
        MenuRow(
          key: ValueKey<String>('send-menu-$key'),
          onPressed: act,
          child: MenuLabel(label, ticked: ticked ?? false),
        );
    showMenuAt<void>(
      context: context,
      position: at,
      width: 160,
      rows: (close) {
        void pre(bool on) {
          close(null);
          if (send.pre == on) return;
          try {
            writeSend(entry.layer, send.bus, pre: on);
          } catch (_) {
            return;
          }
          scope.onChanged();
        }

        return [
          row('pre', l10n.mixerSendPreFader, () => pre(true), ticked: send.pre),
          row('post', l10n.mixerSendPostFader, () => pre(false),
              ticked: !send.pre),
          const MenuDivider(),
          row('mute', l10n.audioSendMute, () {
            close(null);
            try {
              writeSend(entry.layer, send.bus, muted: !send.muted);
            } catch (_) {
              return;
            }
            scope.onChanged();
          }, ticked: send.muted),
          row('remove', l10n.mixerSendRemove, () {
            close(null);
            _routes([
              for (final s in entry.info.sends)
                if (s.bus != send.bus) s,
            ]);
          }),
          row('show', l10n.mixerShowBus, () {
            close(null);
            scope.showBus(send.bus);
          }),
        ];
      },
    );
  }

  /// The buses this strip may send to, asked as the list opens.
  void _add(BuildContext row) {
    final box = row.findRenderObject();
    if (box is! RenderBox) return;
    final self = entry.layer.internallayerId;
    final candidates = [
      for (final bus in scope.buses)
        if (bus.layer.internallayerId != self &&
            !entry.info.sends.any((s) => s.bus == bus.layer.internallayerId))
          bus,
    ];
    showMenuAt<void>(
      context: row,
      position: box.localToGlobal(Offset(0, box.size.height)),
      width: 170,
      rows: (close) => [
        for (final bus in candidates)
          () {
            final id = bus.layer.internallayerId;
            final allowed = scope.comp.routeAllowed(layer: self, bus: id);
            return MenuRow(
              key: ValueKey<String>('send-add-$id'),
              enabled: allowed,
              onPressed: () {
                close(null);
                _routes([
                  ...entry.info.sends,
                  BridgeSend(
                      bus: id,
                      level: const BridgeScalar.static_(0),
                      pre: false),
                ]);
              },
              child: MenuLabel(bus.info.name, enabled: allowed),
            );
          }(),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final names = {
      for (final b in scope.buses) b.layer.internallayerId: b.info.name,
    };
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final send in entry.info.sends)
          _RackRow(
            key: ValueKey<String>('send-${send.bus}'),
            onMenu: (at) => _menu(context, at, send),
            child: _SendLine(
              scope: scope,
              entry: entry,
              send: send,
              name: names[send.bus] ?? '',
            ),
          ),
        _addRow('send-add-${entry.layer.internallayerId}',
            l10n.audioRouteSendTo, _add),
      ],
    );
  }
}

/// A send's row: the bus name with the Pre chip, and the level bar under it,
/// dragged sideways to set the level.
class _SendLine extends StatefulWidget {
  final RackScope scope;
  final BridgeLayerEntry entry;
  final BridgeSend send;
  final String name;

  const _SendLine({
    required this.scope,
    required this.entry,
    required this.send,
    required this.name,
  });

  @override
  State<_SendLine> createState() => _SendLineState();
}

class _SendLineState extends State<_SendLine> {
  final DragEscape _escape = DragEscape();
  double? _drag;
  double _from = 0;

  BridgeMixTarget get _target => BridgeMixTarget.send(
      layer: widget.entry.layer.internallayerId, bus: widget.send.bus);

  /// The level as it stands: where the automation has it while the comp
  /// plays, else the model's at the playhead.
  double _level() {
    final sends = widget.scope.positions
        .value[widget.entry.layer.internallayerId.toString()]
        ?.sends;
    final at =
        widget.entry.info.sends.indexWhere((s) => s.bus == widget.send.bus);
    if (sends != null && at >= 0 && at < sends.length) return sends[at];
    final level = widget.send.level;
    if (level is BridgeScalar_Static) return level.field0;
    return sampledScalar(level,
        timeOfFrame(widget.scope.comp, widget.scope.ui.playheadFrame.value));
  }

  @override
  void dispose() {
    _escape.dispose();
    super.dispose();
  }

  void _commit(double db) {
    final comp = widget.scope.comp;
    mixerTouch(comp, _target, db);
    mixerRelease(comp, _target, widget.scope.ui);
    setState(() => _drag = null);
    widget.scope.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = widget.scope.ui;
    return ListenableBuilder(
      // A keyed level reads at the playhead; a static one never moves with it.
      listenable: Listenable.merge([
        widget.scope.positions,
        if (widget.send.level is! BridgeScalar_Static) ui.playheadFrame,
      ]),
      builder: (context, _) {
        final db = _drag ?? _level();
        // A muted send is still there, and reads as a muted layer does.
        final muted = widget.send.muted;
        return LayoutBuilder(
          builder: (context, box) => GestureDetector(
            behavior: HitTestBehavior.opaque,
            onHorizontalDragStart: (_) {
              final old = _level();
              _from = faderFraction(old);
              _escape.begin(() {
                mixerCancel(widget.scope.comp, _target, old, widget.scope.ui);
                setState(() => _drag = null);
              });
            },
            onHorizontalDragUpdate: (d) {
              if (!_escape.running) return;
              _from += d.delta.dx / box.maxWidth * dragScale();
              final v = faderDb(_from);
              mixerTouch(widget.scope.comp, _target, v);
              setState(() => _drag = v);
            },
            onHorizontalDragEnd: (_) {
              if (_escape.end() && _drag != null) _commit(_drag!);
            },
            onHorizontalDragCancel: () {
              if (_escape.end()) setState(() => _drag = null);
            },
            onDoubleTap: () => _commit(0),
            child: LumitTooltip(
              message: l10n.audioSendTo(widget.name),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(children: [
                    Expanded(
                      child: Text(widget.name,
                          maxLines: 1,
                          softWrap: false,
                          overflow: TextOverflow.ellipsis,
                          style: t.caption.copyWith(
                              fontSize: 8,
                              color: muted ? t.textDisabled : t.textSecondary)),
                    ),
                    if (widget.send.pre)
                      Text(l10n.audioSendPre,
                          style: t.caption.copyWith(
                              fontSize: 8,
                              color: muted ? t.textDisabled : t.accent)),
                  ]),
                  const SizedBox(height: 1),
                  Container(
                    height: 2,
                    color: t.surface0,
                    alignment: Alignment.centerLeft,
                    child: FractionallySizedBox(
                      widthFactor: faderFraction(db),
                      child: Container(
                          color: muted
                              ? t.warning
                              : db == 0
                                  ? t.textSecondary
                                  : t.accent),
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
}

/// One undo step over [targets]' writes, for the menus that act on several.
void eachAsOne(LumitState app, Iterable<BridgeLayerEntry> targets,
    void Function(BridgeLayerEntry e) write) {
  asOneUndoStep(app.project, () {
    for (final e in targets) {
      try {
        write(e);
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
  });
}
