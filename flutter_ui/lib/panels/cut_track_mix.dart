// The Cut timeline's track mix: the small sliders Vegas puts in a track's
// header. A sound track carries Vol and Pan, a picture track Level (its
// opacity). Each is a thin bar with its value written inside it. Drag across
// it to change it, hold Ctrl to move at a tenth of the speed, double-click to
// put it back. The value shows live and is written once, on release, and at
// the playhead when the property is keyed, as the Mixer's fader writes it.

import 'dart:math' as math;

import 'package:flutter/gestures.dart' show kSecondaryMouseButton;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/number_dialog_frb.dart' show askNumberFrb;
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../widgets/controls.dart';
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'mixer_panel_frb.dart' show panLabel;
import 'timeline_extras_frb.dart' show showMenuAt;
import 'volume_band_frb.dart'
    show BandKind, volumeBandFloorDb, volumeBandTopDb;

/// How tall one bar is, and the gap between two.
const double _barHeight = 15;
const double _barGap = 2;

/// The press a bar opened its own menu with, so the head behind it opens none.
int? cutMixMenuPointer;

/// One track's mix controls, filling the width its parent gives it.
class CutTrackMix extends StatelessWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

  /// Vol and Pan when true, Level when false.
  final bool sound;
  final VoidCallback onChanged;

  /// Vol and Pan side by side on one bar's height, for a track too short to
  /// stack them.
  final bool inline;

  /// Whether a sound track's envelope is out across its lane, and the switch
  /// for it, for the bar's own menu.
  final bool Function(BandKind kind) bandShown;
  final void Function(BandKind kind) onBand;

  const CutTrackMix({
    super.key,
    required this.comp,
    required this.entry,
    required this.sound,
    required this.onChanged,
    required this.bandShown,
    required this.onBand,
    this.inline = false,
  });

  /// The height the strip needs, so a header can tell whether it fits.
  static double heightFor(bool sound, {bool inline = false}) =>
      sound && !inline ? _barHeight * 2 + _barGap : _barHeight;

  @override
  Widget build(BuildContext context) {
    final info = entry.info;
    final scalars =
        sound ? [info.volumeDb, info.pan] : [info.transform.opacity];
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    // Only a keyed value moves with the playhead, so only then does the strip
    // listen to it.
    if (scalars.every((s) => s is BridgeScalar_Static)) {
      return _bars(context, playhead.value);
    }
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) => _bars(context, frame),
    );
  }

  Widget _bars(BuildContext context, int frame) {
    final t = ThemeScope.of(context).theme;
    final info = entry.info;
    final layer = entry.layer;
    final id = layer.internallayerId;

    double at(BridgeScalar s) => s is BridgeScalar_Static
        ? s.field0
        : sampledScalar(s, timeOfFrame(comp, frame));

    void write(BridgeScalar was, double v, void Function(BridgeScalar) set) {
      try {
        set(scalarWithValueAt(was, v, comp, frame));
      } catch (_) {
        // A locked layer refuses, and quietly.
        return;
      }
      onChanged();
    }

    // A locked track, or a value an expression drives, is shown and not
    // dragged.
    bool live(BridgeScalar s) =>
        !info.switches.locked && s is! BridgeScalar_Expression;

    if (!sound) {
      final level = info.transform.opacity;
      return _MixBar(
        key: ValueKey<String>('ctl-mix-level-$id'),
        label: l10n.cutMixLevel,
        value: at(level),
        min: 0,
        max: 100,
        reset: 100,
        fill: t.accent,
        enabled: live(level),
        suffix: l10n.unitSymbolPercent,
        text: (v) => '${v.round()}${l10n.unitSymbolPercent}',
        onCommit: (v) => write(
            level,
            v,
            (s) => layer.setTransform(
                prop: BridgeTransformProp.opacity, value: s)),
      );
    }
    final volume = info.volumeDb;
    final pan = info.pan;
    final bars = [
      _MixBar(
        key: ValueKey<String>('ctl-mix-volume-$id'),
        label: l10n.cutMixVolume,
        value: at(volume),
        min: volumeBandFloorDb,
        max: volumeBandTopDb,
        reset: 0,
        fill: t.accent,
        enabled: live(volume),
        text: (v) => v <= volumeBandFloorDb
            ? l10n.volumeNegInf
            : '${v.toStringAsFixed(1)} ${l10n.unitSymbolDb}',
        onCommit: (v) => write(volume, v, (s) => layer.setVolumeDb(value: s)),
        suffix: l10n.unitSymbolDb,
        band: (
          label: l10n.cutVolumeEnvelope,
          action: 'cut.envelope.volume',
          shown: bandShown(BandKind.volume),
          toggle: () => onBand(BandKind.volume),
        ),
      ),
      // The pan band's colour, so the bar and the line on the lane match.
      _MixBar(
        key: ValueKey<String>('ctl-mix-pan-$id'),
        label: l10n.audioPan,
        value: at(pan),
        min: -100,
        max: 100,
        reset: 0,
        origin: 0,
        fill: t.warning,
        enabled: live(pan),
        text: panLabel,
        onCommit: (v) => write(pan, v, (s) => layer.setPan(value: s)),
        band: (
          label: l10n.cutPanEnvelope,
          action: 'cut.envelope.pan',
          shown: bandShown(BandKind.pan),
          toggle: () => onBand(BandKind.pan),
        ),
      ),
    ];
    if (inline) {
      return Row(children: [
        Expanded(child: bars[0]),
        const SizedBox(width: _barGap * 2),
        Expanded(child: bars[1]),
      ]);
    }
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [bars[0], const SizedBox(height: _barGap), bars[1]],
    );
  }
}

/// One thin bar: the fill runs from [origin] to the value, the label and the
/// value are written over it.
class _MixBar extends StatefulWidget {
  final String label;
  final double value;
  final double min;
  final double max;
  final double reset;

  /// Where the fill starts: the left end, or centre for Pan.
  final double? origin;
  final Color fill;
  final String Function(double) text;
  final ValueChanged<double> onCommit;

  /// False shows the value dimmed and takes no drag.
  final bool enabled;

  /// The unit the number box asks in.
  final String? suffix;

  /// The envelope that draws this same value across the lane, for the menu.
  final ({String label, String action, bool shown, VoidCallback toggle})? band;

  const _MixBar({
    super.key,
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.reset,
    required this.fill,
    required this.text,
    required this.onCommit,
    required this.enabled,
    this.origin,
    this.suffix,
    this.band,
  });

  @override
  State<_MixBar> createState() => _MixBarState();
}

class _MixBarState extends State<_MixBar> {
  /// The value while a drag runs, null at rest.
  double? _live;

  /// A right click on the bar: type the value, put it back, and the envelope
  /// that draws it across the lane.
  void _menu(Offset at) {
    final band = widget.band;
    final changed = widget.value != widget.reset;
    showMenuAt<void>(
      context: context,
      position: at,
      width: 210,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('ctl-mix-set'),
          onPressed: () async {
            close(null);
            final typed = await askNumberFrb(
              context: context,
              title: widget.label,
              value: widget.value,
              min: widget.min,
              max: widget.max,
              decimals: 1,
              suffix: widget.suffix,
            );
            if (typed != null && mounted && typed != widget.value) {
              widget.onCommit(typed);
            }
          },
          child: MenuLabel(l10n.envelopePointSet,
              ticked: band == null ? null : false),
        ),
        MenuRow(
          key: const ValueKey('ctl-mix-reset'),
          enabled: changed,
          onPressed: () {
            close(null);
            widget.onCommit(widget.reset);
          },
          child: MenuLabel(l10n.reset,
              enabled: changed, ticked: band == null ? null : false),
        ),
        if (band != null) ...[
          const MenuDivider(),
          MenuRow(
            key: const ValueKey('ctl-mix-band'),
            onPressed: () {
              close(null);
              band.toggle();
            },
            child: MenuLabel(band.label,
                action: band.action, ticked: band.shown),
          ),
        ],
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final shown = _live ?? widget.value;
    // At its usual value the bar recedes, so a track that has been turned
    // down or panned is the one that stands out.
    final changed = (shown - widget.reset).abs() > 1e-6;
    final value = t.mono.copyWith(
        fontSize: 9, color: changed ? t.textPrimary : t.textMuted);
    final fill = changed ? widget.fill : t.textMuted;
    final held = changed || _live != null;
    return LayoutBuilder(builder: (context, box) {
      final width = box.maxWidth.isFinite ? box.maxWidth : 120.0;
      double xOf(double v) =>
          ((v - widget.min) / (widget.max - widget.min)).clamp(0.0, 1.0) *
          width;
      final from = xOf(widget.origin ?? widget.min);
      final to = xOf(shown);
      final bar = HoverMark(
        cursor: SystemMouseCursors.resizeLeftRight,
        builder: (context, lit) {
          // A value at its usual place is its name alone until the pointer
          // comes, so eight rows at full say nothing eight times.
          final bar = held ? 1.0 : lit;
          return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onHorizontalDragStart: (_) => setState(() => _live = widget.value),
          onHorizontalDragUpdate: (d) {
            final keyboard = HardwareKeyboard.instance;
            final fine = keyboard.isControlPressed || keyboard.isMetaPressed;
            // The bar's width is its whole range; Ctrl is a tenth of that.
            final per = (widget.max - widget.min) / math.max(1.0, width);
            final by = d.delta.dx * per * (fine ? 0.1 : 1);
            setState(() => _live =
                ((_live ?? widget.value) + by).clamp(widget.min, widget.max));
          },
          onHorizontalDragEnd: (_) {
            final v = _live;
            setState(() => _live = null);
            if (v != null && v != widget.value) widget.onCommit(v);
          },
          onHorizontalDragCancel: () => setState(() => _live = null),
          onDoubleTap: () {
            if (widget.value != widget.reset) widget.onCommit(widget.reset);
          },
          child: Container(
            width: width,
            height: _barHeight,
            clipBehavior: Clip.antiAlias,
            decoration: BoxDecoration(
              color: t.surface2.withValues(alpha: t.surface2.a * bar),
              borderRadius: BorderRadius.circular(2),
            ),
            child: Stack(fit: StackFit.expand, children: [
              Positioned(
                left: math.min(from, to),
                width: (to - from).abs(),
                top: 0,
                bottom: 0,
                // Lifts a shade under the pointer.
                child: ColoredBox(
                    color: fill.withValues(
                        alpha: ((changed ? 0.25 : 0.1) + 0.2 * lit) * bar)),
              ),
              // The origin, so Pan at centre reads as a centred slider.
              if (widget.origin != null)
                Positioned(
                  left: from - 0.5,
                  width: 1,
                  top: 0,
                  bottom: 0,
                  child: ColoredBox(
                      color: t.textMuted.withValues(alpha: t.textMuted.a * bar)),
                ),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 5),
                child: Row(children: [
                  // Too narrow for both, the value wins once there is one to show.
                  if (width >= 72 || bar == 0) ...[
                    Text(widget.label,
                        style: t.small.copyWith(color: t.textMuted),
                        maxLines: 1),
                    const SizedBox(width: 4),
                  ],
                  Expanded(
                    child: Opacity(
                      opacity: bar,
                      child: Text(
                        widget.text(shown),
                        style: value,
                        textAlign: TextAlign.right,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ),
                ]),
              ),
            ]),
          ),
          );
        },
      );
      return IgnorePointer(
        ignoring: !widget.enabled,
        child: Opacity(
          opacity: widget.enabled ? 1 : 0.5,
          child: Listener(
            onPointerDown: (e) {
              if (e.buttons != kSecondaryMouseButton) return;
              cutMixMenuPointer = e.pointer;
              _menu(e.position);
            },
            child: bar,
          ),
        ),
      );
    });
  }
}
