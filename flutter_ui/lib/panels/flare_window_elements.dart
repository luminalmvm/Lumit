// The Flare window's left side: the list of elements in draw order, the Add
// menu, and the glyph each kind is drawn with.

part of 'flare_window.dart';

extension _Elements on _FlareWindowState {
  Widget _elementsPane(LumitTheme t) {
    // The one Add button: in the header, or in the middle of an empty list.
    final add = KeyedSubtree(
      key: _addKey,
      child: HouseButton(
        key: const ValueKey('flare-add'),
        small: true,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        onPressed: _openAddMenu,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            glyph.LumitIcon(LumitIcons.addEffect,
                size: 9, colour: t.textPrimary),
            const SizedBox(width: 5),
            Text(l10n.flareAdd),
          ],
        ),
      ),
    );
    final empty = _elements.isEmpty;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 8, 4),
          child: SizedBox(
            height: 22,
            child: Row(
              children: [
                Expanded(
                  child: Text(t.kickerCase(l10n.flareElements),
                      style: t.kicker, overflow: TextOverflow.ellipsis),
                ),
                if (!empty) add,
              ],
            ),
          ),
        ),
        Expanded(
          child: empty
              ? Center(child: add)
              : ListView(
                  padding: const EdgeInsets.only(bottom: 8),
                  children: [
                    for (final (i, e) in _elements.indexed)
                      _elementRow(t, i, e),
                  ],
                ),
        ),
      ],
    );
  }

  String _kindLabel(String kind) {
    for (final k in _kinds) {
      if (k.key == kind) return engineLabel(k.label);
    }
    return kind;
  }

  Widget _elementRow(LumitTheme t, int i, Json e) {
    final on = i == _selected;
    // A design written by hand may leave "on" out, and the engine reads that
    // as on.
    final visible = e['on'] != false;
    final shown = visible && (_solo == null || _solo == i);
    final kind = e['kind'] as String;
    final dim = t.textMuted;
    final ink = shown ? t.textPrimary : dim;
    return HouseContextMenu(
      itemBuilder: (close) => [
        MenuRow(
          key: ValueKey('flare-element-duplicate-$i'),
          onPressed: () {
            close();
            _duplicateElement(i);
          },
          child: Text(l10n.flareDuplicate),
        ),
        MenuRow(
          key: ValueKey('flare-element-delete-$i'),
          onPressed: () {
            close();
            _deleteElement(i);
          },
          child: Text(l10n.flareDelete),
        ),
      ],
      child: GestureDetector(
        key: ValueKey('flare-element-$i'),
        behavior: HitTestBehavior.opaque,
        // A second click on the same row inside a moment renames it. Timed
        // by hand rather than onDoubleTap, which would hold every single
        // click back until the double-tap window had passed.
        onTapDown: (_) {
          final now = DateTime.now();
          final again = _lastTap == i &&
              now.difference(_lastTapAt) < const Duration(milliseconds: 350);
          _lastTap = i;
          _lastTapAt = now;
          if (again) {
            _redraw(() {
              _renaming = i;
              _listName.text = (e['name'] as String?) ?? '';
            });
          } else {
            _select(i);
          }
        },
        child: Container(
          height: _rowHeight,
          // Always a colour, so selecting another row mid-drag keeps the
          // grip's gesture alive rather than rebuilding it.
          color: on ? t.selectionFill : t.selectionFill.withValues(alpha: 0),
          child: Row(
            children: [
              Container(width: 2, color: on ? t.accent : null),
              // The grip: drag it up or down to change the draw order.
              GestureDetector(
                key: ValueKey('flare-grip-$i'),
                behavior: HitTestBehavior.opaque,
                onVerticalDragStart: (_) {
                  _undo.add((jsonEncode(_design), _selected));
                  _redo.clear();
                  _coalesce = null;
                  _dragFrom = i;
                  _dragAt = i;
                },
                onVerticalDragUpdate: (d) {
                  final from = _dragFrom;
                  if (from == null) return;
                  _dragDy += d.delta.dy;
                  final to = (from + (_dragDy / _rowHeight).round())
                      .clamp(0, _elements.length - 1);
                  if (to != _dragAt) {
                    _redraw(() => _moveElement(_dragAt!, to));
                    _dragAt = to;
                  }
                },
                onVerticalDragEnd: (_) => _endDrag(),
                onVerticalDragCancel: _endDrag,
                child: MouseRegion(
                  cursor: SystemMouseCursors.grab,
                  child: SizedBox(
                    width: 18,
                    height: _rowHeight,
                    child: CustomPaint(painter: _GripPainter(dim)),
                  ),
                ),
              ),
              // The eye: on or off. Alt-click shows this one alone in the
              // preview, and again brings the rest back.
              GestureDetector(
                key: ValueKey('flare-eye-$i'),
                behavior: HitTestBehavior.opaque,
                onTap: () {
                  if (HardwareKeyboard.instance.isAltPressed) {
                    _redraw(() => _solo = _solo == i ? null : i);
                    _requestPreview();
                  } else {
                    _edit(() => e['on'] = !visible);
                  }
                },
                child: HoverMark(
                  builder: (context, lit) => SizedBox(
                    width: 20,
                    height: _rowHeight,
                    child: Center(
                      child: glyph.LumitIcon(
                        visible
                            ? LumitIcons.visible
                            : LumitIcons.hidden,
                        size: 11,
                        colour: Color.lerp(
                            _solo == i ? t.accent : ink, t.textPrimary, lit),
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 2),
              SizedBox(
                width: 16,
                height: 16,
                child: CustomPaint(painter: _KindGlyph(kind, ink)),
              ),
              const SizedBox(width: 7),
              Expanded(
                child: _renaming == i
                    ? HouseTextField(
                        key: ValueKey('flare-element-name-$i'),
                        controller: _listName,
                        width: double.infinity,
                        autofocus: true,
                        submitOnLostFocus: true,
                        onSubmitted: (text) {
                          final name = text.trim();
                          _renaming = null;
                          if (name.isEmpty || name == e['name']) {
                            _redraw(() {});
                          } else {
                            _edit(() => e['name'] = name);
                            _syncName();
                          }
                        },
                      )
                    : Text((e['name'] as String?) ?? '',
                        style: t.body.copyWith(color: ink),
                        overflow: TextOverflow.ellipsis),
              ),
              const SizedBox(width: 6),
              Text(_kindLabel(kind),
                  style: t.small.copyWith(color: dim),
                  overflow: TextOverflow.ellipsis),
              const SizedBox(width: 10),
            ],
          ),
        ),
      ),
    );
  }

  void _endDrag() {
    _dragFrom = null;
    _dragAt = null;
    _dragDy = 0;
    _requestPreview();
    _drawViewer();
  }

  Future<void> _openAddMenu() async {
    final box = _addKey.currentContext?.findRenderObject();
    if (box is! RenderBox) return;
    final t = ThemeScope.of(context).theme;
    final places = <String>[];
    for (final k in _kinds) {
      if (!places.contains(k.place)) places.add(k.place);
    }
    final kind = await showLumitPopup<String>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height + 4)),
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final place in places) ...[
                Padding(
                  padding: const EdgeInsets.fromLTRB(10, 8, 10, 2),
                  child: Text(t.kickerCase(engineLabel(place)),
                      style: t.kicker),
                ),
                for (final k in _kinds)
                  if (k.place == place)
                    MenuRow(
                      key: ValueKey('flare-add-${k.key}'),
                      onPressed: () => close(k.key),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          SizedBox(
                            width: 16,
                            height: 16,
                            child: CustomPaint(
                                painter: _KindGlyph(k.key, t.textPrimary)),
                          ),
                          const SizedBox(width: 8),
                          Padding(
                            padding: const EdgeInsets.only(right: 12),
                            child: Text(engineLabel(k.label)),
                          ),
                        ],
                      ),
                    ),
              ],
              const SizedBox(height: 4),
            ],
          ),
        ),
      ),
    );
    if (kind != null && mounted) _addKind(kind);
  }
}

/// Two columns of three dots.
class _GripPainter extends CustomPainter {
  final Color colour;
  const _GripPainter(this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()..color = colour.withValues(alpha: 0.6);
    final c = size.center(Offset.zero);
    for (final x in [-2.0, 2.0]) {
      for (final y in [-4.0, 0.0, 4.0]) {
        canvas.drawCircle(c + Offset(x, y), 1, p);
      }
    }
  }

  @override
  bool shouldRepaint(_GripPainter old) => old.colour != colour;
}

/// A kind's glyph, drawn to fit the box it is given.
class _KindGlyph extends CustomPainter {
  final String kind;
  final Color colour;
  const _KindGlyph(this.kind, this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.shortestSide / 2 - 1;
    final fill = Paint()..color = colour;
    final faint = Paint()..color = colour.withValues(alpha: 0.35);
    final line = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.2;
    Path polygon(Offset at, double radius, int sides) {
      final p = Path();
      for (var i = 0; i < sides; i++) {
        final a = -math.pi / 2 + i * 2 * math.pi / sides;
        final o = at + Offset(math.cos(a), math.sin(a)) * radius;
        i == 0 ? p.moveTo(o.dx, o.dy) : p.lineTo(o.dx, o.dy);
      }
      return p..close();
    }

    void rays(int n, double length, double width, {double turn = 0}) {
      final p = Paint()
        ..color = colour
        ..strokeWidth = width
        ..strokeCap = StrokeCap.round;
      for (var i = 0; i < n; i++) {
        final a = turn + i * math.pi / (n / 2);
        canvas.drawLine(c, c + Offset(math.cos(a), math.sin(a)) * length, p);
      }
    }

    switch (kind) {
      case 'glow':
        canvas.drawCircle(c, r, faint);
        canvas.drawCircle(c, r * 0.45, fill);
      case 'rays':
        rays(12, r, 0.8);
      case 'star':
        rays(6, r, 1.6);
      case 'streak':
        canvas.drawLine(
            c - Offset(r, 0), c + Offset(r, 0), line..strokeWidth = 2);
        canvas.drawCircle(c, 2, fill);
      case 'ring':
        canvas.drawCircle(c, r - 0.5, line);
      case 'halo':
        canvas.drawArc(Rect.fromCircle(center: c, radius: r - 0.5),
            math.pi * 0.75, math.pi * 1.2, false, line..strokeWidth = 1.6);
      case 'ghost':
        canvas.drawPath(polygon(c, r - 0.5, 6), line);
      case 'ghosts':
        canvas.drawPath(polygon(c + Offset(-r * 0.5, r * 0.5), r * 0.42, 6), line);
        canvas.drawPath(polygon(c, r * 0.3, 6), line);
        canvas.drawPath(polygon(c + Offset(r * 0.55, -r * 0.5), r * 0.36, 6), line);
      case 'orbs':
        canvas.drawCircle(c + Offset(-r * 0.45, -r * 0.3), r * 0.42, fill);
        canvas.drawCircle(c + Offset(r * 0.45, r * 0.35), r * 0.32, fill);
        canvas.drawCircle(c + Offset(r * 0.25, -r * 0.6), r * 0.2, faint);
      case 'dirt':
        for (final (x, y, s) in [
          (-0.6, -0.5, 0.18),
          (0.3, -0.7, 0.12),
          (0.6, 0.1, 0.2),
          (-0.2, 0.3, 0.1),
          (-0.5, 0.7, 0.14),
          (0.2, 0.7, 0.16),
        ]) {
          canvas.drawCircle(c + Offset(x * r, y * r), s * r + 0.4, fill);
        }
      case 'picture':
        final box = Rect.fromCenter(center: c, width: r * 2, height: r * 1.6);
        canvas.drawRRect(RRect.fromRectAndRadius(box, const Radius.circular(1.5)), line);
        canvas.drawCircle(box.topLeft + Offset(r * 0.6, r * 0.5), r * 0.22, fill);
        final hill = Path()
          ..moveTo(box.left + 1, box.bottom - 1)
          ..lineTo(box.left + r * 0.9, box.top + r * 0.8)
          ..lineTo(box.left + r * 1.3, box.top + r * 1.15)
          ..lineTo(box.right - 1, box.top + r * 0.6)
          ..lineTo(box.right - 1, box.bottom - 1)
          ..close();
        canvas.drawPath(hill, faint);
      default:
        canvas.drawCircle(c, r * 0.4, fill);
    }
  }

  @override
  bool shouldRepaint(_KindGlyph old) =>
      old.kind != kind || old.colour != colour;
}
