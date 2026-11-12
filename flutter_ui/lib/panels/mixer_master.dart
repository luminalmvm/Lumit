// The Master strip: the fader is a gain stage on the sum, keyable like any
// other, the lamp is the limiter's, and the loudness stands beside the meter
// with the comp's target.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/number_dialog_frb.dart';
import '../state/app_state.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import '../widgets/controls.dart';
import 'audio_loudness.dart';
import 'audio_routes_frb.dart' show automationModeLabel;
import 'audio_meters_feed.dart';
import 'loudness_commands.dart';
import 'mixer_live.dart';
import 'mixer_meter.dart';
import 'mixer_panel_frb.dart' show meterRows;
import 'mixer_strip.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

/// The column beside the Master's meter that holds its loudness.
const double _loudnessColumn = 56;

class MasterStrip extends StatefulWidget {
  final CompositionReference comp;
  final AudioMeterFeed feed;
  final double width;

  /// How tall the strips' racks stand, so this fader starts level with theirs.
  final double rackHeight;

  /// Whether the loudness has room to stand beside the meter, rather than the
  /// integrated number alone under the level.
  final bool loudness;
  final VoidCallback onChanged;

  const MasterStrip({
    super.key,
    required this.comp,
    required this.feed,
    required this.width,
    required this.rackHeight,
    required this.loudness,
    required this.onChanged,
  });

  @override
  State<MasterStrip> createState() => _MasterStripState();
}

class _MasterStripState extends State<MasterStrip> {
  double? _dragDb;
  double _fromDb = 0;

  /// The master's fader, mode and target, read when the document moves and
  /// never while drawing.
  BridgeScalar _volume = const BridgeScalar.static_(0);
  BridgeAutomationMode _mode = BridgeAutomationMode.read;
  BridgeLoudnessTarget? _target;

  late final LumitUiState _ui = context.read<LumitUiState>();

  @override
  void initState() {
    super.initState();
    _ui.model.addListener(_read);
    _read();
  }

  @override
  void didUpdateWidget(MasterStrip old) {
    super.didUpdateWidget(old);
    if (old.comp != widget.comp) _read();
  }

  @override
  void dispose() {
    _ui.model.removeListener(_read);
    super.dispose();
  }

  void _read() {
    try {
      final volume = widget.comp.masterVolume();
      final mode = widget.comp.masterAutomationMode();
      final target = widget.comp.loudnessTarget();
      if (volume == _volume && mode == _mode && target == _target) return;
      _volume = volume;
      _mode = mode;
      _target = target;
    } catch (_) {
      return;
    }
    if (mounted) setState(() {});
  }

  int get _frame => _ui.playheadFrame.value;

  double _shown() =>
      _dragDb ??
      widget.feed.positions.value['']?.volumeDb ??
      (_volume is BridgeScalar_Static
          ? (_volume as BridgeScalar_Static).field0
          : sampledScalar(_volume, timeOfFrame(widget.comp, _frame)));

  void _live(double db) {
    if (_dragDb == null) _fromDb = _shown();
    setState(() => _dragDb = db);
    mixerTouch(widget.comp, const BridgeMixTarget.master(), db);
  }

  void _commit(double db) {
    mixerTouch(widget.comp, const BridgeMixTarget.master(), db);
    mixerRelease(widget.comp, const BridgeMixTarget.master(), _ui);
    setState(() => _dragDb = null);
    widget.onChanged();
  }

  void _cancel() {
    mixerCancel(widget.comp, const BridgeMixTarget.master(), _fromDb, _ui);
    setState(() => _dragDb = null);
  }

  void _menu(Offset at) {
    final db = _shown();
    final app = context.read<LumitState>();
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
            _commit(0);
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
                min: faderFloorDb,
                max: 12,
                decimals: 1,
                suffix: ' ${l10n.unitSymbolDb}');
            if (typed != null && mounted) _commit(typed);
          },
          child: MenuLabel(l10n.mixerSetLevel),
        ),
        const MenuDivider(),
        ...loudnessMenuRows(
          context: context,
          app: app,
          comp: widget.comp,
          current: _target,
          close: () => close(null),
          onChanged: widget.onChanged,
          tickColumn: false,
        ),
        const MenuDivider(),
        ...meterRows(_ui, () => close(null)),
      ],
    );
  }

  void _modeMenu(BuildContext chip) {
    final box = chip.findRenderObject();
    if (box is! RenderBox) return;
    showMenuAt<void>(
      context: chip,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      width: 130,
      rows: (close) => automationRows(_mode, (mode) {
        close(null);
        try {
          widget.comp.setMasterAutomationMode(mode: mode);
        } catch (_) {
          return;
        }
        widget.onChanged();
      }),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final keyed = _volume is BridgeScalar_Keyframed;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (e) => _menu(e.globalPosition),
      child: Container(
        width: widget.width,
        decoration: BoxDecoration(
          color: t.surface1,
          border: Border(left: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Column(
          children: [
            SizedBox(
              height: stripNameRow,
              child: Text(
                l10n.mixerMaster,
                style: t.small.copyWith(color: t.textPrimary),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(height: stripGap),
            SizedBox(
                height: stripPotRow,
                child: Center(child: _LimiterLamp(feed: widget.feed))),
            const SizedBox(height: stripGap),
            SizedBox(height: widget.rackHeight),
            const SizedBox(height: stripGap),
            SizedBox(
              height: stripChipRow,
              child: Padding(
                padding: const EdgeInsets.only(left: 2),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: MixChip(
                    key: const ValueKey('mode-master'),
                    label: automationModeLabel(_mode),
                    ink: automationInk(t, _mode, keyed),
                    quiet: _mode == BridgeAutomationMode.read,
                    tip: l10n.audioAutomation,
                    onTap: _modeMenu,
                  ),
                ),
              ),
            ),
            const SizedBox(height: stripGap),
            Expanded(
              child: Row(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _following(
                      keyed,
                      () => MixFader(
                            key: const ValueKey('fader-master'),
                            db: _shown(),
                            recording: widget.feed.recording.value
                                .contains(const BridgeMixTarget.master()),
                            onLive: _live,
                            onCommit: _commit,
                            onCancel: _cancel,
                          )),
                  const SizedBox(width: 3),
                  MeterBand(stripId: '', feed: widget.feed),
                  // Beside the meter, its top level with the fader's.
                  if (widget.loudness) ...[
                    const SizedBox(width: 5),
                    SizedBox(
                      width: _loudnessColumn,
                      // A short desk cuts the readings rather than the meter.
                      child: SingleChildScrollView(
                        physics: const NeverScrollableScrollPhysics(),
                        child: LoudnessReadout(
                            feed: widget.feed,
                            layout: LoudnessLayout.column,
                            target: _target?.lufs),
                      ),
                    ),
                  ],
                ],
              ),
            ),
            const SizedBox(height: stripGap),
            _following(keyed, () => DbWell(db: _shown(), onCommit: _commit)),
            const SizedBox(height: stripGap),
            // With no room beside the meter, the integrated loudness stands
            // where a strip has its switches.
            SizedBox(
              height: stripSwitchRow,
              child: widget.loudness
                  ? null
                  : Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 4),
                      // A long target shrinks the line rather than cut it.
                      child: FittedBox(
                        fit: BoxFit.scaleDown,
                        child: LoudnessReadout(
                            feed: widget.feed,
                            layout: LoudnessLayout.integrated,
                            target: _target?.lufs),
                      ),
                    ),
            ),
            const SizedBox(height: stripGap),
            // Where a strip has its output: the short-term loudness, across,
            // so it never reads against the fader's dB.
            SizedBox(
              height: stripChipRow,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 6),
                child: _LoudnessBar(feed: widget.feed, target: _target?.lufs),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// Rebuilt as the fader moves with the automation, with the playhead when
  /// the master is keyed, and as a pass starts and stops writing it.
  Widget _following(bool keyed, Widget Function() build) => ListenableBuilder(
        listenable: Listenable.merge([
          widget.feed.positions,
          widget.feed.recording,
          if (keyed) _ui.playheadFrame,
        ]),
        builder: (context, _) => build(),
      );
}

/// The master's short-term loudness at the strip's foot: the reading, and a
/// slim bar under it on a loudness scale of its own from
/// [_LoudnessBarPainter.floor] to 0 LUFS, with the comp's loudness target as
/// a mark across it. Within a unit of the
/// target the bar takes the success colour, louder the warning colour.
class _LoudnessBar extends StatelessWidget {
  final AudioMeterFeed feed;
  final double? target;

  const _LoudnessBar({required this.feed, required this.target});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final mark = t.mono.copyWith(fontSize: 9, color: t.textMuted, height: 1);
    final aim = target;
    return LumitTooltip(
      message: l10n.loudnessShortTerm,
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          // The reading, as the integrated one above it stands, read against
          // the target the same way.
          ValueListenableBuilder<LoudnessLevels>(
            valueListenable: feed.loudness,
            builder: (context, l, _) {
              final s = l.shortTerm;
              final ink = !s.isFinite
                  ? t.textMuted
                  : aim != null && (s - aim).abs() <= 1
                      ? t.success
                      : aim != null && s > aim
                          ? t.warning
                          : t.textSecondary;
              return Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Text(l10n.loudnessShortTermMark, style: mark),
                  const SizedBox(width: 4),
                  Text(
                    s.isFinite ? s.toStringAsFixed(1) : l10n.volumeNegInf,
                    key: const ValueKey('master-short-term'),
                    maxLines: 1,
                    softWrap: false,
                    style: mark.copyWith(color: ink),
                  ),
                ],
              );
            },
          ),
          const SizedBox(height: 1),
          RepaintBoundary(
            child: CustomPaint(
              size: const Size.fromHeight(4),
              painter: _LoudnessBarPainter(
                loudness: feed.loudness,
                target: target,
                trough: t.surface0,
                quiet: t.textMuted,
                near: t.success,
                over: t.warning,
                mark: t.textPrimary,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _LoudnessBarPainter extends CustomPainter {
  final ValueListenable<LoudnessLevels> loudness;
  final double? target;
  final Color trough, quiet, near, over, mark;

  _LoudnessBarPainter({
    required this.loudness,
    required this.target,
    required this.trough,
    required this.quiet,
    required this.near,
    required this.over,
    required this.mark,
  }) : super(repaint: loudness);

  /// The quietest loudness the bar shows, in LUFS.
  static const double floor = -48;

  /// Where [lufs] stands along a bar [width] wide.
  static double _x(double width, double lufs) =>
      width * (1 - (lufs / floor).clamp(0.0, 1.0));

  @override
  void paint(Canvas canvas, Size size) {
    const bar = 2.0;
    final y = (size.height - bar) / 2;
    canvas.drawRect(
        Rect.fromLTWH(0, y, size.width, bar), Paint()..color = trough);
    final aim = target;
    final s = loudness.value.shortTerm;
    if (s.isFinite && s > floor) {
      final ink = aim == null
          ? quiet
          : (s - aim).abs() <= 1
              ? near
              : s > aim
                  ? over
                  : quiet;
      canvas.drawRect(Rect.fromLTWH(0, y, _x(size.width, s), bar),
          Paint()..color = ink);
    }
    if (aim != null) {
      canvas.drawRect(
          Rect.fromLTWH(_x(size.width, aim) - 0.5, 0, 1, size.height),
          Paint()..color = mark);
    }
  }

  @override
  bool shouldRepaint(_LoudnessBarPainter old) =>
      old.target != target ||
      old.loudness != loudness ||
      old.near != near ||
      old.trough != trough;
}

/// The limiter's lamp: LIM, lit once the limiter has held the master down or
/// the sum has reached the ceiling. Clicking it puts the light out; the mix is
/// untouched. Its own small widget under the lamp's listenable, so a meter
/// tick never rebuilds it.
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
                  color: Color.lerp(
                      lit ? t.error : t.textMuted, t.textPrimary, 0.35 * over),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
