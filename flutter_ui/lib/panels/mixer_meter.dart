// The Mixer's small controls: the meters with their clip lamp and the
// gain-reduction bar, the fader, the pots, the dB well, the switches and the
// chips a strip wears. Each draws what it is handed and hands its gestures
// back; the strip decides what they write.

import 'dart:math' as math;

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../l10n/strings.dart';
import '../shell/dialog_frame.dart' show FittedText;
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import 'audio_meters_feed.dart';
import 'mixer_panel_frb.dart' show panLabel;

/// The fader's floor: the engine's own knee, at or below which the sound is
/// exactly off (`lumit_audio::mix::VOLUME_FLOOR_DB`).
const double faderFloorDb = -100.0;

/// The fader's taper, as a console prints it: points of travel against dB,
/// silence at the bottom, unity two thirds of the way up and +12, the most
/// the value field takes, at the top, with most of the throw spent near unity.
const List<(double, double)> _taper = [
  (0.0, faderFloorDb),
  (0.05, -60),
  (0.14, -40),
  (0.23, -30),
  (0.345, -20),
  (0.5, -10),
  (0.69, 0),
  (0.85, 6),
  (1.0, 12),
];

/// Where [db] sits on the fader's travel, nought to one.
double faderFraction(double db) {
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
double faderDb(double fraction) {
  final f = fraction.clamp(0.0, 1.0);
  for (var i = 1; i < _taper.length; i++) {
    final (f0, d0) = _taper[i - 1];
    final (f1, d1) = _taper[i];
    if (f <= f1) return d0 + (f - f0) / (f1 - f0) * (d1 - d0);
  }
  return _taper.last.$2;
}

/// How much a drag moves a control while Ctrl is held.
double dragScale() => HardwareKeyboard.instance.isControlPressed ? 0.1 : 1;

/// How tall the knob is drawn.
const double _knobHeight = 7;

/// Where [db] stands on a fader or meter [height] tall, so the scale, the
/// knob, the bars and the Master's loudness all read off one ruler.
double faderY(double height, double db) =>
    _knobHeight / 2 + (height - _knobHeight) * (1 - faderFraction(db));

/// The meter's own width, and the gain-reduction bar's beside it.
const double _meterWidth = 11;
const double _reductionWidth = 2;

/// One strip's stereo meters: RMS body, peak behind, and the panel's own
/// hold line, on the fader's scale beside them, with the gain-reduction bar
/// at the right.
///
/// A [RepaintBoundary] around a [CustomPaint] whose painter repaints off the
/// feed's frame notifier directly: a meter tick is a repaint of this band
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
        width: _meterWidth + 1 + _reductionWidth,
        child: CustomPaint(
          painter: MeterPainter(
            frame: feed.frame,
            stripId: stripId,
            trough: t.surface0,
            edge: t.hairline,
            low: t.success,
            mid: t.warning,
            high: t.error,
            hold: t.textPrimary,
            reduction: t.accent,
          ),
        ),
      ),
    );
  }
}

class MeterPainter extends CustomPainter {
  final ValueListenable<AudioMeterFrame> frame;
  final String stripId;
  final Color trough, edge, low, mid, high, hold, reduction;

  MeterPainter({
    required this.frame,
    required this.stripId,
    required this.trough,
    required this.edge,
    required this.low,
    required this.mid,
    required this.high,
    required this.hold,
    required this.reduction,
  }) : super(repaint: frame);

  /// On the fader's own scale, so the numbers printed beside the rail read
  /// the meter too: a bar level with the 0 mark is at 0 dB.
  double _yOfDb(Size size, double db) => faderY(size.height, db);

  @override
  void paint(Canvas canvas, Size size) {
    final levels = frame.value.of(stripId);
    const barWidth = 4.0;
    const gap = 3.0;
    final left = (_meterWidth - barWidth * 2 - gap) / 2;
    _bar(canvas, size, left, levels.rmsLeft, levels.peakLeft, levels.holdLeft);
    _bar(canvas, size, left + barWidth + gap, levels.rmsRight, levels.peakRight,
        levels.holdRight);
    // Held down by a duck or the limiter: a thin bar growing down from 0 dB.
    final gr = levels.gainReductionDb;
    if (gr > 0) {
      final top = _yOfDb(size, 0);
      canvas.drawRect(
        Rect.fromLTRB(_meterWidth + 1, top, size.width,
            _yOfDb(size, -gr.clamp(0.0, 60.0))),
        Paint()..color = reduction,
      );
    }
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
    double yOf(double amplitude) {
      // Under the meter's floor is silence, and draws nothing.
      final db = amplitudeDb(amplitude);
      return db <= -60 ? size.height : _yOfDb(size, db);
    }

    // Green to -18 dBFS, amber to -6, red above, as the bar climbs.
    final yMid = _yOfDb(size, -18);
    final yHigh = _yOfDb(size, -6);
    void zones(double top, double alpha) {
      if (top >= size.height) return;
      void band(double from, double to, Color colour) {
        final a = math.max(top, to);
        if (a >= from) return;
        canvas.drawRect(Rect.fromLTRB(x, a, x + barWidth, from),
            Paint()..color = colour.withValues(alpha: alpha));
      }

      band(size.height, yMid, low);
      band(yMid, yHigh, mid);
      band(yHigh, 0, high);
    }

    // Peak behind, dimmer; RMS in front. The bar's body is what it sounds
    // like, the dim part above it the loudest instant.
    zones(yOf(peak), 0.45);
    zones(yOf(rms), 1);
    final y = yOf(holdValue);
    if (y < size.height) {
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
      old.stripId != stripId || old.frame != frame || old.low != low;
}

/// A strip's clip light, over its meter: lit once the strip's sound reaches
/// full scale, until clicked or every peak is reset.
class ClipLamp extends StatelessWidget {
  final String stripId;
  final AudioMeterFeed feed;

  const ClipLamp({super.key, required this.stripId, required this.feed});

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    return ValueListenableBuilder<Set<String>>(
      valueListenable: feed.clips,
      builder: (context, clips, _) {
        final lit = clips.contains(stripId);
        final lamp = GestureDetector(
          key: ValueKey('clip-lamp-$stripId'),
          behavior: HitTestBehavior.opaque,
          onTap: lit ? () => feed.clearClip(stripId) : null,
          child: AnimatedContainer(
            duration: scope.motion.mark.duration,
            curve: scope.motion.mark.curve,
            width: _meterWidth,
            height: 4,
            decoration: BoxDecoration(
              color: lit ? t.error : t.surface0,
              border: Border.all(color: lit ? t.error : t.hairline),
              borderRadius: BorderRadius.circular(1),
            ),
          ),
        );
        return lit
            ? LumitTooltip(message: l10n.mixerClipped, child: lamp)
            : lamp;
      },
    );
  }
}

/// The fader: a vertical drag over a rail, with the console's scale printed
/// beside it. The knob moves by as much as the pointer does, wherever it was
/// taken hold of, so a press a hair off the knob never jumps the level. Every
/// move goes up through [onLive], the release through [onCommit] and Escape
/// through [onCancel]; double-click asks for unity through [onCommit].
class MixFader extends StatefulWidget {
  final double db;
  final ValueChanged<double> onLive;
  final ValueChanged<double> onCommit;
  final VoidCallback onCancel;

  /// Whether an automation pass is writing this fader, which turns the knob
  /// to the error colour.
  final bool recording;

  const MixFader({
    super.key,
    required this.db,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
    this.recording = false,
  });

  @override
  State<MixFader> createState() => _MixFaderState();
}

/// The marks the scale prints, as a console's are: no minus signs, the floor
/// left to the rail's own foot.
const List<(double, String)> _faderScale = [
  (12, '12'),
  (6, '6'),
  (0, '0'),
  (-10, '10'),
  (-20, '20'),
  (-40, '40'),
];

class _MixFaderState extends State<MixFader> with TickerProviderStateMixin {
  late final HoverGlow _glow = HoverGlow(this, () => setState(() {}));
  final DragEscape _escape = DragEscape();
  double _from = 0;
  double _travel = 0;

  @override
  void dispose() {
    _escape.dispose();
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
              _from = faderFraction(widget.db);
              _travel = 0;
              _glow.mark(motion, held: true);
              _escape.begin(() {
                _glow.mark(motion, held: false);
                widget.onCancel();
              });
            },
            onVerticalDragUpdate: (d) {
              if (!_escape.running) return;
              _travel += d.delta.dy * dragScale();
              widget.onLive(faderDb(_from - _travel / throw_));
            },
            // The strip fed [db] every live tick, so what is showing is what
            // the release commits.
            onVerticalDragEnd: (_) {
              _glow.mark(motion, held: false);
              if (_escape.end()) widget.onCommit(widget.db);
            },
            onVerticalDragCancel: () {
              _glow.mark(motion, held: false);
              if (_escape.end()) widget.onCancel();
            },
            onDoubleTap: () => widget.onCommit(0.0),
            child: SizedBox(
              width: 26,
              child: CustomPaint(
                painter: _FaderPainter(
                  db: widget.db,
                  rail: t.surface3,
                  // A fader at unity is where it usually rests, so it stays
                  // quiet, one that has been moved takes the accent, and one
                  // being written takes the error colour.
                  knob: Color.lerp(
                      widget.recording
                          ? t.error
                          : widget.db == 0
                              ? t.textSecondary
                              : t.accent,
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
  final double db;
  final Color rail, knob, tick;
  final TextStyle label;

  const _FaderPainter({
    required this.db,
    required this.rail,
    required this.knob,
    required this.tick,
    required this.label,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // The rail stands at the right of the box, the scale's words to its left.
    final x = size.width - 7;
    canvas.drawRect(
      Rect.fromLTWH(x - 1.5, 0, 3, size.height),
      Paint()..color = rail,
    );
    // On a short strip the numbers would run into each other, so any too near
    // the last one printed gives way, working down from the top.
    var lastY = double.negativeInfinity;
    for (final (db, text) in _faderScale) {
      final y = faderY(size.height, db);
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
      words.paint(canvas, Offset(x - 9 - words.width, y - words.height / 2));
    }
    final y = faderY(size.height, db);
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
      old.db != db ||
      old.knob != knob ||
      old.rail != rail ||
      old.tick != tick;
}

/// A small dial, -[range] through centre to +[range]: the pan pot by default,
/// and the Gain pot with a range of 24 dB. Vertical drag turns it, Ctrl
/// finely; double-click recentres; Escape puts it back. Public because the
/// Audio panel draws the same pot on its Pan row, and one dial must turn the
/// same way everywhere.
class PanPot extends StatefulWidget {
  final double value;
  final ValueChanged<double> onLive;
  final ValueChanged<double> onCommit;
  final VoidCallback onCancel;
  final double range;

  /// What hovering it says, the pan reading when null.
  final String? tip;

  const PanPot({
    super.key,
    required this.value,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
    this.range = 100,
    this.tip,
  });

  @override
  State<PanPot> createState() => _PanPotState();
}

class _PanPotState extends State<PanPot> {
  final DragEscape _escape = DragEscape();
  double _live = 0;

  @override
  void dispose() {
    _escape.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final range = widget.range;
    return LumitTooltip(
      message: widget.tip ?? panLabel(widget.value),
      child: HoverMark(
        cursor: SystemMouseCursors.resizeUpDown,
        builder: (context, lit) => GestureDetector(
          behavior: HitTestBehavior.opaque,
          onVerticalDragStart: (_) {
            _live = widget.value;
            _escape.begin(widget.onCancel);
          },
          onVerticalDragUpdate: (d) {
            if (!_escape.running) return;
            _live = (_live - d.delta.dy * range / 50 * dragScale())
                .clamp(-range, range);
            widget.onLive(_live);
          },
          onVerticalDragEnd: (_) {
            if (_escape.end()) widget.onCommit(widget.value);
          },
          onVerticalDragCancel: () {
            if (_escape.end()) widget.onCancel();
          },
          onDoubleTap: () => widget.onCommit(0.0),
          child: SizedBox(
            width: 14,
            height: 14,
            child: CustomPaint(
              painter: _PotPainter(
                fraction: (widget.value + range) / (2 * range),
                ring: Color.lerp(t.hairlineStrong, t.textSecondary, lit)!,
                mark: Color.lerp(widget.value == 0 ? t.textSecondary : t.accent,
                    t.textPrimary, lit * 0.3)!,
              ),
            ),
          ),
        ),
      ),
    );
  }
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
    // Straight up at centre, 135 degrees either way at the ends: a pot's throw.
    final angle = (fraction - 0.5) * 1.5 * math.pi;
    final dir = Offset(math.sin(angle), -math.cos(angle)) * (radius * 0.9);
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
class DbWell extends StatefulWidget {
  final double db;
  final ValueChanged<double> onCommit;

  const DbWell({super.key, required this.db, required this.onCommit});

  @override
  State<DbWell> createState() => _DbWellState();
}

class _DbWellState extends State<DbWell> {
  bool _typing = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final shown = widget.db <= faderFloorDb
        ? l10n.volumeNegInf
        : widget.db.toStringAsFixed(1);
    return Container(
      width: 50,
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
                min: faderFloorDb,
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

class SwitchGlyph extends StatelessWidget {
  final String keyName;

  /// One of the set's glyphs, or one of the older marks through [icon].
  final String? mark;
  final LumitIcon? icon;
  final bool on;
  final String tip;
  final VoidCallback onTap;

  const SwitchGlyph({
    super.key,
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

/// The S switch. A solo safe strip wears a ring round it.
class SoloChip extends StatelessWidget {
  final String keyName;
  final bool on;
  final bool safe;
  final VoidCallback onTap;

  const SoloChip({
    super.key,
    required this.keyName,
    required this.on,
    required this.safe,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    return LumitTooltip(
      message: safe
          ? l10n.audioSoloSafe
          : on
              ? l10n.switchSoloed
              : l10n.switchSolo,
      child: HoverMark(
        builder: (context, lit) => GestureDetector(
          key: ValueKey(keyName),
          onTap: onTap,
          behavior: HitTestBehavior.opaque,
          child: AnimatedContainer(
            duration: scope.motion.mark.duration,
            curve: scope.motion.mark.curve,
            padding: const EdgeInsets.all(1),
            decoration: BoxDecoration(
              border: Border.all(
                  color: safe ? t.accent : t.accent.withValues(alpha: 0)),
              borderRadius: BorderRadius.circular(3),
            ),
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
      ),
    );
  }
}

/// A strip's small chip: the automation mode, or the output. Its [ink] eases
/// as what it names changes. A [width] makes it a well across the strip, as
/// the output stands; without one it is a pill as wide as its word, washed in
/// its ink unless [quiet], as the mode sits beside the clip lamp.
class MixChip extends StatelessWidget {
  final String label;
  final Color ink;
  final double? width;
  final bool quiet;
  final String? tip;
  final void Function(BuildContext context)? onTap;

  const MixChip({
    super.key,
    required this.label,
    required this.ink,
    this.width,
    this.quiet = false,
    this.tip,
    this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final well = width != null;
    final chip = HoverMark(
      cursor: onTap == null ? MouseCursor.defer : SystemMouseCursors.click,
      builder: (context, lit) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap == null ? null : () => onTap!(context),
        child: TweenAnimationBuilder<Color?>(
          tween: ColorTween(end: ink),
          duration: scope.motion.mark.duration,
          curve: scope.motion.mark.curve,
          builder: (context, colour, _) => Container(
            width: width,
            height: 14,
            padding: const EdgeInsets.symmetric(horizontal: 3),
            decoration: BoxDecoration(
              color: well
                  ? Color.lerp(t.surface2, t.surface3, lit)
                  : Color.lerp(
                      (colour ?? ink).withValues(alpha: quiet ? 0 : 0.16),
                      t.surface3,
                      lit * 0.6),
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
            // A pill is as wide as its word; a well centres it, and a word
            // cut short there says the whole of itself on hover.
            child: Align(
              widthFactor: well ? null : 1,
              child: FittedText(
                label,
                t.caption.copyWith(
                    color: Color.lerp(colour, t.textPrimary, 0.3 * lit)),
              ),
            ),
          ),
        ),
      ),
    );
    return tip == null ? chip : LumitTooltip(message: tip!, child: chip);
  }
}
