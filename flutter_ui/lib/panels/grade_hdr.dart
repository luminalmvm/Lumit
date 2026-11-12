// The Wheels page's HDR mode: six tonal zones, each with its own colour,
// exposure and saturation, counted in stops from mid grey, and the Global
// balance, Black offset and Falloff they share.
//
// The strip over the wheels shows where each zone sits: one line a zone, how
// much of it reaches each stop, with its range as a handle to drag.

part of 'grade_panel_frb.dart';

/// The zones, darkest first. Black, Dark and Shadow reach from their range
/// down; Light, Highlight and Specular from theirs up.
const List<String> _hdrZones = [
  'black',
  'dark',
  'shadow',
  'light',
  'highlight',
  'specular',
];

final List<String> _hdrRows = [
  'hdr_balance',
  'hdr_black_offset',
  'hdr_falloff',
  for (final z in _hdrZones) ...[
    'hdr_${z}_balance',
    'hdr_${z}_exposure',
    'hdr_${z}_saturation',
    'hdr_${z}_range',
  ],
];

/// How far either side of grey the strip reads, in stops.
const double _hdrSpan = 8;

mixin _GradeHdr on _GradeModel, _GradePrimaries {
  static String _zoneName(String z) => switch (z) {
        'black' => l10n.gradeZoneBlack,
        'dark' => l10n.gradeZoneDark,
        'shadow' => l10n.gradeZoneShadow,
        'light' => l10n.gradeZoneLight,
        'highlight' => l10n.gradeZoneHighlight,
        _ => l10n.gradeZoneSpecular,
      };

  /// The zone whose range is being dragged on the strip.
  String? _zoneHeld;

  /// The range in hand while the strip's drag runs, in stops.
  double _zoneAt = 0;

  /// Seven cells: the six zones and Global. As many across as have room for
  /// their three numbers, then fewer.
  @override
  Widget _hdrBody(LumitTheme t, bool enabled) => Padding(
        padding: const EdgeInsets.symmetric(horizontal: gradeInset),
        child: LayoutBuilder(builder: (context, box) {
          const gap = 8.0;
          // Three fields of six characters each, as Exp, Sat and Range are.
          final well =
              monoSlotWidth(t.mono.copyWith(fontSize: wellTextSize), 6) + 14;
          final least = math.max(104.0, 3 * well + 8);
          final across = [7, 4, 3, 2, 1].firstWhere(
              (n) => (box.maxWidth - gap * (n - 1)) / n >= least,
              orElse: () => 1);
          final cell = (box.maxWidth - gap * (across - 1)) / across;
          final size = (cell - 16).clamp(48.0, _hdrWheelMax).toDouble();
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _zoneStrip(t, enabled),
              const SizedBox(height: 8),
              Wrap(
                spacing: gap,
                runSpacing: 10,
                children: [
                  for (final z in _hdrZones)
                    SizedBox(
                        width: cell,
                        child: _zoneCell(t, z, cell, size, enabled)),
                  SizedBox(
                      width: cell, child: _globalCell(t, cell, size, enabled)),
                ],
              ),
            ],
          );
        }),
      );

  /// A balance wheel with no master: the zones keep their brightness, so
  /// only the lean means anything. A stop a channel at the rim's half.
  Widget _hdrWheel(String id, double size, bool enabled) => GradeMenu(
        rows: (close) => _menuFor(close, rows: [id], param: id),
        child: GradeWheel(
          key: ValueKey<String>('grade-wheel-$id'),
          rgb: _colour(id),
          neutral: 0,
          reach: 0.5,
          masterSpan: 0.5,
          size: size,
          bar: false,
          enabled: enabled,
          keyed: _keyed(id),
          onStart: _touchStart,
          onLive: (c) => _live(id, _colourFor(id, c)),
          onCommit: (c) => _write(id, (on) => _colourFor(id, c, on)),
          onCancel: _touchCancel,
        ),
      );

  Widget _cellName(
          LumitTheme t, String name, List<String> rows, bool enabled) =>
      GradeMenu(
        rows: (close) => _menuFor(close, rows: rows),
        child: Text(
          name,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: t.small.copyWith(
            color: rows.any(_keyed)
                ? t.animated
                : !enabled
                    ? t.textDisabled
                    : _anyChanged(rows)
                        ? t.textSecondary
                        : t.textMuted,
          ),
        ),
      );

  Widget _zoneCell(
      LumitTheme t, String z, double cell, double size, bool enabled) {
    final rows = [
      'hdr_${z}_balance',
      'hdr_${z}_exposure',
      'hdr_${z}_saturation',
      'hdr_${z}_range',
    ];
    final field = (cell - 8) / 3;
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        _hdrWheel(rows[0], size, enabled),
        const SizedBox(height: 4),
        _cellName(t, _zoneName(z), rows, enabled),
        const SizedBox(height: 2),
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            _namedField(t, rows[1], enabled,
                width: field, label: l10n.gradeZoneExposure),
            const SizedBox(width: 4),
            _namedField(t, rows[2], enabled,
                width: field, label: l10n.gradeZoneSaturation, decimals: 0),
            const SizedBox(width: 4),
            _namedField(t, rows[3], enabled,
                width: field, label: l10n.gradeZoneRange),
          ],
        ),
      ],
    );
  }

  Widget _globalCell(LumitTheme t, double cell, double size, bool enabled) {
    const rows = ['hdr_balance', 'hdr_black_offset', 'hdr_falloff'];
    final field = (cell - 4) / 2;
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        _hdrWheel(rows[0], size, enabled),
        const SizedBox(height: 4),
        _cellName(t, l10n.gradeZoneGlobal, rows, enabled),
        const SizedBox(height: 2),
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            _namedField(t, rows[1], enabled, width: field, decimals: 3),
            const SizedBox(width: 4),
            _namedField(t, rows[2], enabled, width: field),
          ],
        ),
      ],
    );
  }

  /// How big a zone wheel may be: one row of cells and the zone strip in
  /// the pane's height, with each cell's name and numbers.
  double get _hdrWheelMax {
    final room = _room;
    if (room == null) return 96;
    return (room.height - _stripHeight - 112).clamp(48.0, 96.0).toDouble();
  }

  // --- The zone strip ----------------------------------------------------------

  static const double _stripHeight = 46;

  /// Where each zone reaches, -8 to 8 stops across. A drag near a zone's
  /// range handle moves its range, Shift finer; a double-click puts it back.
  Widget _zoneStrip(LumitTheme t, bool enabled) {
    final falloff = _float('hdr_falloff');
    final zones = [
      for (final z in _hdrZones)
        (
          zone: z,
          range: _zoneHeld == z ? _zoneAt : _float('hdr_${z}_range'),
          down: _hdrZones.indexOf(z) < 3,
          lit: _zoneHeld == z,
          changed: enabled &&
              _anyChanged([
                'hdr_${z}_balance',
                'hdr_${z}_exposure',
                'hdr_${z}_saturation',
                'hdr_${z}_range',
              ]),
        ),
    ];
    return LayoutBuilder(builder: (context, box) {
      final width = box.maxWidth;
      double xOf(double stops) =>
          (stops.clamp(-_hdrSpan, _hdrSpan) + _hdrSpan) /
          (2 * _hdrSpan) *
          width;
      String? nearest(Offset p) {
        String? best;
        var far = 10.0;
        for (final z in zones) {
          final d = (xOf(z.range) - p.dx).abs();
          if (d < far) {
            far = d;
            best = z.zone;
          }
        }
        return best;
      }

      final painter = _ZoneStripPainter(
        zones: zones,
        falloff: falloff,
        line: t.textMuted,
        changed: t.textSecondary,
        lit: t.accent,
        hairline: t.hairline,
        ground: t.surface0,
        label: t.caption.copyWith(color: t.textMuted),
        enabled: enabled,
      );
      final strip = CustomPaint(
        key: const ValueKey('grade-hdr-zones'),
        size: Size(width, _stripHeight),
        painter: painter,
      );
      if (!enabled) return strip;
      return GestureDetector(
        behavior: HitTestBehavior.opaque,
        // The handle under the press, not where the drag was recognised.
        dragStartBehavior: DragStartBehavior.down,
        onDoubleTapDown: (d) {
          final z = nearest(d.localPosition);
          if (z != null) _reset(['hdr_${z}_range']);
        },
        onHorizontalDragStart: (d) {
          final z = nearest(d.localPosition);
          if (z == null) return;
          _touchStart();
          setState(() {
            _zoneHeld = z;
            _zoneAt = _float('hdr_${z}_range');
          });
        },
        onHorizontalDragUpdate: (d) {
          final z = _zoneHeld;
          if (z == null || width <= 0) return;
          final rate = HardwareKeyboard.instance.isShiftPressed ? 0.1 : 1.0;
          final (lo, hi) = _hardRange('hdr_${z}_range');
          setState(() => _zoneAt =
              (_zoneAt + d.delta.dx / width * 2 * _hdrSpan * rate)
                  .clamp(lo, hi)
                  .toDouble());
          _liveFloat('hdr_${z}_range', _zoneAt);
        },
        onHorizontalDragEnd: (_) {
          final z = _zoneHeld;
          if (z == null) return;
          final at = _zoneAt;
          setState(() => _zoneHeld = null);
          _writeFloat('hdr_${z}_range', at);
        },
        onHorizontalDragCancel: () {
          if (_zoneHeld == null) return;
          setState(() => _zoneHeld = null);
          _touchCancel();
        },
        child: MouseRegion(
            cursor: SystemMouseCursors.resizeLeftRight, child: strip),
      );
    });
  }
}

/// The zone strip: a stop scale, one line a zone for how much of the zone
/// reaches each stop, and a handle at each zone's range.
class _ZoneStripPainter extends CustomPainter {
  final List<({String zone, double range, bool down, bool lit, bool changed})>
      zones;
  final double falloff;
  final Color line, changed, lit, hairline, ground;
  final TextStyle label;
  final bool enabled;

  const _ZoneStripPainter({
    required this.zones,
    required this.falloff,
    required this.line,
    required this.changed,
    required this.lit,
    required this.hairline,
    required this.ground,
    required this.label,
    required this.enabled,
  });

  static double _smooth(double a, double b, double x) {
    if (b <= a) return x < a ? 0 : 1;
    final t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    return t * t * (3 - 2 * t);
  }

  /// The engine's weight: down zones fade out above their range, up zones
  /// fade in above theirs.
  double _weight(double range, bool down, double s) => down
      ? 1 - _smooth(range - falloff, range, s)
      : _smooth(range, range + falloff, s);

  @override
  void paint(Canvas canvas, Size size) {
    const top = 8.0, scale = 12.0;
    final bottom = size.height - scale;
    final w = size.width;
    double xOf(double s) => (s + _hdrSpan) / (2 * _hdrSpan) * w;
    final hair = Paint()
      ..color = hairline
      ..strokeWidth = 1;
    canvas.drawLine(Offset(0, bottom), Offset(w, bottom), hair);
    for (var s = -_hdrSpan.toInt(); s <= _hdrSpan; s++) {
      final x = xOf(s.toDouble());
      final major = s % 4 == 0;
      canvas.drawLine(
          Offset(x, bottom), Offset(x, bottom + (major ? 3 : 2)), hair);
      if (!major) continue;
      final text = TextPainter(
        text: TextSpan(text: s > 0 ? '+$s' : '$s', style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      final dx = (x - text.width / 2).clamp(0.0, w - text.width);
      text.paint(canvas, Offset(dx, bottom + 2));
    }
    // Grey, where Shadow and Light meet.
    canvas.drawLine(Offset(xOf(0), top), Offset(xOf(0), bottom), hair);
    // The lit zone last, so it is drawn over the rest.
    final order = [...zones]..sort((a, b) => (a.lit ? 1 : 0) - (b.lit ? 1 : 0));
    for (final z in order) {
      final colour = !enabled
          ? line.withValues(alpha: 0.5)
          : z.lit
              ? lit
              : z.changed
                  ? changed
                  : line;
      final path = Path();
      const steps = 96;
      for (var i = 0; i <= steps; i++) {
        final s = -_hdrSpan + 2 * _hdrSpan * i / steps;
        final y = bottom - (bottom - top) * _weight(z.range, z.down, s);
        i == 0 ? path.moveTo(xOf(s), y) : path.lineTo(xOf(s), y);
      }
      canvas.drawPath(
          path,
          Paint()
            ..style = PaintingStyle.stroke
            ..strokeWidth = z.lit ? 1.5 : 1
            ..color = colour);
      // The range's handle: a small notch on the top edge, pointing the
      // way the zone reaches.
      final x = xOf(z.range.clamp(-_hdrSpan, _hdrSpan));
      final dir = z.down ? -1.0 : 1.0;
      final notch = Path()
        ..moveTo(x, 1)
        ..lineTo(x, top - 1)
        ..lineTo(x + 4 * dir, top - 1)
        ..close();
      canvas.drawPath(notch, Paint()..color = colour);
      canvas.drawLine(
          Offset(x, top),
          Offset(x, bottom),
          Paint()
            ..color = colour.withValues(alpha: 0.35)
            ..strokeWidth = 1);
    }
  }

  @override
  bool shouldRepaint(_ZoneStripPainter old) =>
      old.falloff != falloff ||
      old.enabled != enabled ||
      old.line != line ||
      old.lit != lit ||
      old.zones.length != zones.length ||
      [for (var i = 0; i < zones.length; i++) zones[i] != old.zones[i]]
          .any((x) => x);
}
