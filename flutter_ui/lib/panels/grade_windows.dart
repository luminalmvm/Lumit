// The Grade panel's Windows page: the node's four shapes, each turned on as
// an ellipse, a rectangle, a polygon or a gradient, and tracking for the one
// picked.
//
// The shapes themselves are drawn and moved on the Viewer; here are the
// rows that aren't a place on the picture, and the tracker.

part of 'grade_panel_frb.dart';

mixin _GradeWindows on _GradeModel, _GradePrimaries {
  /// A window track in hand, and how far it has got.
  StreamSubscription<GradeTrackProgress>? _track;
  GradeTrackProgress? _trackAt;

  static const List<String> _slotRows = [
    'x',
    'y',
    'w',
    'h',
    'angle',
    'round',
    'soft',
    'soft_in',
    'opacity',
    'invert',
    'subtract',
    'points',
    'smooth',
  ];

  List<String> _rowsOfSlot(int n) =>
      ['win${n}_shape', for (final r in _slotRows) 'win${n}_$r'];

  /// The slots as a list, with the picked one's rows under it; in a wide
  /// pane the list stands on the left and the rows beside it.
  Widget _windowsPage(LumitTheme t, bool enabled) {
    final picked = ui.grade.window.value;
    final open = picked > 0 && _choice('win${picked}_shape') != 0;
    return LayoutBuilder(builder: (context, box) {
      if (box.maxWidth >= 760) {
        return Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: 260,
                child: Column(
                  children: [
                    for (var n = 1; n <= 4; n++)
                      _slotHeader(t, n, enabled, picked == n),
                  ],
                ),
              ),
              const SizedBox(width: 12),
              if (open)
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: _slotRowsFor(t, picked, enabled),
                  ),
                ),
            ],
          ),
        );
      }
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: 4),
          for (var n = 1; n <= 4; n++) ...[
            _slotHeader(t, n, enabled, picked == n),
            if (picked == n && open) ..._slotRowsFor(t, n, enabled),
          ],
        ],
      );
    });
  }

  /// A slot's line: its number, then a button for each shape, the one in
  /// force lit. Pressing it again turns the slot off. A click on the line
  /// picks the slot, whose handles the Viewer then shows.
  Widget _slotHeader(LumitTheme t, int n, bool enabled, bool picked) {
    final shape = _choice('win${n}_shape');
    final rows = _rowsOfSlot(n);
    const glyphs = [
      LumitIcons.ellipse,
      LumitIcons.rectangle,
      LumitIcons.polygon,
      LumitIcons.gradientWindow,
    ];
    final names = switch (_kind('win${n}_shape')) {
      BridgeParamKind_Choice(:final options) => options,
      _ => const <String>[],
    };
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: rows),
      child: GestureDetector(
        key: ValueKey<String>('grade-window-$n'),
        behavior: HitTestBehavior.opaque,
        onTap: () => ui.grade.window.value = picked ? 0 : n,
        child: Container(
          height: 26,
          margin: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
          padding: const EdgeInsets.only(left: 4, right: 2),
          decoration: BoxDecoration(
            color: picked ? t.surface1 : null,
            borderRadius: BorderRadius.circular(t.tokens.actionRadius),
          ),
          child: Row(
            children: [
              SizedBox(
                width: 64,
                child: Text(
                  l10n.gradeWindowN(n),
                  style: t.small.copyWith(
                      color: !enabled
                          ? t.textDisabled
                          : shape != 0
                              ? t.textSecondary
                              : t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              for (var i = 0; i < glyphs.length; i++)
                GradeIconButton(
                  key: ValueKey<String>('grade-window-$n-shape-${i + 1}'),
                  glyph: glyphs[i],
                  tip: i + 1 < names.length ? engineLabel(names[i + 1]) : '',
                  active: shape == i + 1,
                  onPressed: !enabled
                      ? null
                      : () {
                          final shot = _shot;
                          // Turned on for the first time, it starts in the
                          // middle of the shot's own picture.
                          final placed = shape == 0 &&
                                  shot != null &&
                                  !_anyChanged(['x', 'y', 'w', 'h']
                                      .map((r) => 'win${n}_$r'))
                              ? gradeWindowStart(ui, shot, n)
                              : const <String, BridgeEffectValue>{};
                          _set({
                            ...placed,
                            'win${n}_shape':
                                BridgeEffectValue.choice(shape == i + 1 ? 0 : i + 1)
                          });
                          ui.grade.window.value = shape == i + 1 ? 0 : n;
                        },
                ),
              const Spacer(),
              if (enabled && _anyChanged(rows))
                GradeIconButton(
                  key: ValueKey<String>('grade-window-$n-reset'),
                  glyph: LumitIcons.reset,
                  tip: l10n.reset,
                  onPressed: () => _reset(rows),
                ),
            ],
          ),
        ),
      ),
    );
  }

  List<Widget> _slotRowsFor(LumitTheme t, int n, bool enabled) {
    final shape = _choice('win${n}_shape');
    return [
      _flow([
        _slider(t, 'win${n}_soft', enabled, decimals: 0),
        _slider(t, 'win${n}_soft_in', enabled, decimals: 0),
        _slider(t, 'win${n}_opacity', enabled, decimals: 0),
        if (shape == 2) _slider(t, 'win${n}_round', enabled, decimals: 0),
      ]),
      Wrap(
        spacing: 8,
        children: [
          _toggle(t, 'win${n}_invert', enabled),
          _toggle(t, 'win${n}_subtract', enabled),
          if (shape == 3) _toggle(t, 'win${n}_smooth', enabled),
        ],
      ),
      _trackRow(t, n, enabled),
    ];
  }

  /// Track the picked window back to the start of the shot or forwards to
  /// its end, with Stop and how far it has got while it runs.
  Widget _trackRow(LumitTheme t, int n, bool enabled) {
    // A group's or shared node's window is every member's, so it can't
    // follow one shot's picture.
    final ready = enabled &&
        _GradeModel._engine.trackingReady &&
        gradeScopeOf(ui) == null;
    final at = _trackAt;
    final running = _track != null;
    return Padding(
      padding: const EdgeInsets.fromLTRB(gradeInset, 4, 4, 6),
      child: Row(
        children: [
          SizedBox(
            width: 74,
            child: Text(l10n.gradeTrack,
                style: t.small
                    .copyWith(color: ready ? t.textMuted : t.textDisabled)),
          ),
          GradeIconButton(
            key: const ValueKey('grade-track-back'),
            glyph: LumitIcons.previousFrame,
            tip: l10n.gradeTrackBack,
            onPressed: ready && !running
                ? () => _trackStart(n, GradeTrackDirection.backward)
                : null,
          ),
          GradeIconButton(
            key: const ValueKey('grade-track-stop'),
            glyph: LumitIcons.stop,
            tip: l10n.gradeTrackStop,
            onPressed: running ? _trackStop : null,
          ),
          GradeIconButton(
            key: const ValueKey('grade-track-forwards'),
            glyph: LumitIcons.play,
            tip: l10n.gradeTrackForwards,
            onPressed: ready && !running
                ? () => _trackStart(n, GradeTrackDirection.forward)
                : null,
          ),
          const SizedBox(width: 8),
          if (running)
            Expanded(
              child: HouseProgressBar(
                fraction:
                    at == null || at.total == 0 ? 0 : at.done / at.total,
              ),
            ),
        ],
      ),
    );
  }

  void _trackStart(int slot, GradeTrackDirection direction) {
    final shot = _shot;
    final comp = ui.selectedComp;
    final node = _node;
    if (shot == null || comp == null || node == null) return;
    final frame = ui.playheadFrame.value;
    setState(() {
      _trackAt = null;
      _track = _GradeModel._engine
          .trackWindow(
              comp, shot, node.id, slot, frame, direction)
          .listen(
        (p) {
          if (!mounted) return;
          setState(() => _trackAt = p);
          if (p.lostAt case final frame?) {
            app.postNotice(l10n.gradeTrackLost(frame));
          }
        },
        onDone: _trackDone,
        onError: (_) => _trackDone(),
        cancelOnError: true,
      );
    });
  }

  void _trackDone() {
    _track = null;
    _trackAt = null;
    if (!mounted) return;
    setState(() {});
    _tell();
  }

  /// Stop the track: the frames done so far are kept. [quiet] from
  /// `dispose`, where nothing may rebuild.
  void _trackStop({bool quiet = false}) {
    final track = _track;
    if (track == null) return;
    _GradeModel._engine.stopTracking();
    track.cancel();
    if (quiet) {
      _track = null;
      return;
    }
    _trackDone();
  }
}
