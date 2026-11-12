// The Grade panel on a node that isn't a Correction: a 0.11 Colour balance, a
// LUT, a Film. Its own rows, drawn by Effect controls' row, so every node of
// a grade can be worked here. A CDL lays out as slope, offset and power, and
// the Colour warper shows its mesh.

part of 'grade_panel_frb.dart';

mixin _GradeGeneric on _GradeModel, _GradePrimaries {
  Widget _generic(LumitTheme t) {
    final node = _node;
    final comp = ui.selectedComp;
    final shot = _shot;
    if (node == null || comp == null || shot == null) {
      return const SizedBox.shrink();
    }
    final heading = GradeMenu(
      rows: _groundMenu,
      child: GradeHeading(
        label: _shared?.name ??
            node.customName ??
            engineLabel(_GradePanelFrbState._effectLabel(node.name)),
        changed: _nodeChanged,
        onReset: _resetNode,
      ),
    );
    if (node.name == 'cdl') return _cdlPage(t, heading, node.enabled);
    if (node.name == colourWarperName) {
      return _warperPage(t, heading, node.enabled);
    }
    final values = {for (final v in node.values) v.id: v.value};
    final layers = ui.model.layers;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        heading,
        for (final p in _schema.values)
          if (!p.derived)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: gradeInset),
              child: EffectParamRowFrb(
                key: ValueKey<String>('grade-generic-${node.id}-${p.id}'),
                effectId: node.id,
                param: p,
                value: _editor.stagedValue(node.id, p.id) ?? values[p.id],
                comp: comp,
                playheadFrame: ui.playheadFrame.value,
                onSeek: (frame) => ui.playheadFrame.value = frame,
                ownerLayerId: shot.layer.internallayerId,
                ownerLayers: layers,
                enabled: node.enabled,
                // The node is there already, so a drag adds nothing and
                // needs no group of its own; the release is the one step.
                onLive: (_, param, value) => _live(param, value),
                onWrite: (_, param, value) {
                  _touchStart();
                  _write(param, (_) => value, all: false);
                },
                onWritePair: (_, pair) {
                  _touchStart();
                  _writeMany(
                      {for (final e in pair.entries) e.key: (_) => e.value},
                      all: false);
                },
              ),
            ),
      ],
    );
  }

  /// A choice row as a segmented control, one step a press.
  Widget _choiceRow(LumitTheme t, String id, bool enabled) {
    final options = switch (_kind(id)) {
      BridgeParamKind_Choice(:final options) => options,
      _ => const <String>[],
    };
    final at = _choice(id);
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: [id]),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(gradeInset, 4, gradeInset, 4),
        child: Row(
          children: [
            SizedBox(
              width: 74,
              child: Text(_label(id),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: t.small.copyWith(
                      color: !enabled
                          ? t.textDisabled
                          : _changed(id)
                              ? t.textSecondary
                              : t.textMuted)),
            ),
            Flexible(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Segment(children: [
                  for (var i = 0; i < options.length; i++)
                    SegmentOption(
                      key: ValueKey<String>('grade-choice-$id-$i'),
                      active: i == at,
                      onTap: enabled && i != at
                          ? () => _set({id: BridgeEffectValue.choice(i)})
                          : () {},
                      child: Text(engineLabel(options[i]),
                          style: t.small.copyWith(
                              color: i == at
                                  ? t.textPrimary
                                  : t.textSecondary)),
                    ),
                ]),
              ),
            ),
          ],
        ),
      ),
    );
  }

  // --- CDL -----------------------------------------------------------------------

  /// Slope, offset and power as a number a channel, then saturation and mix.
  Widget _cdlPage(LumitTheme t, Widget heading, bool enabled) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          heading,
          _choiceRow(t, 'space', enabled),
          const SizedBox(height: 4),
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 420),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: gradeInset),
                child: Column(
                  children: [
                    for (final id in const ['slope', 'offset', 'power'])
                      _cdlRow(t, id, enabled),
                  ],
                ),
              ),
            ),
          ),
          const SizedBox(height: 4),
          _flow([
            _slider(t, 'saturation', enabled),
            _slider(t, 'mix', enabled),
          ]),
        ],
      );

  Widget _cdlRow(LumitTheme t, String id, bool enabled) => GradeMenu(
        rows: (close) => _menuFor(close, rows: [id], param: id),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 2),
          child: Row(
            children: [
              SizedBox(
                width: 74,
                child: Text(_label(id),
                    style: t.small.copyWith(
                        color: _keyed(id)
                            ? t.animated
                            : !enabled
                                ? t.textDisabled
                                : _changed(id)
                                    ? t.textSecondary
                                    : t.textMuted),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
              ),
              for (var i = 0; i < 3; i++)
                Expanded(
                    child:
                        _mixerField(id, i, enabled, signed: id == 'offset')),
            ],
          ),
        ),
      );

  // --- Colour warper -------------------------------------------------------------

  List<Offset> get _warperMoves => switch (_value('points')) {
        BridgeEffectValue_Shape(:final field0) => [
            for (final p in field0)
              if (p.length >= 2) Offset(p[0], p[1])
          ],
        _ => const [],
      };

  static BridgeEffectValue _shapeOf(List<Offset> moves) =>
      BridgeEffectValue.shape([
        for (final m in moves) Float32List.fromList([m.dx, m.dy])
      ]);

  /// The mesh with its rows beside it, or under it in a narrow pane.
  Widget _warperPage(LumitTheme t, Widget heading, bool enabled) {
    final hues = _choice('resolution') == 1 ? 8 : 6;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        heading,
        LayoutBuilder(builder: (context, box) {
          final room = _room;
          final tall = room == null ? 320.0 : room.height - 40;
          final side = box.maxWidth >= 640;
          final size = math
              .min(side ? box.maxWidth * 0.45 : box.maxWidth - 2 * gradeInset,
                  side ? tall : 360.0)
              .clamp(160.0, 360.0)
              .toDouble();
          final mesh = GradeMenu(
            rows: (close) => _menuFor(close, rows: const ['points']),
            child: GradeWarperMesh(
              key: const ValueKey('grade-warper'),
              hues: hues,
              moves: _warperMoves,
              size: size,
              enabled: enabled,
              onStart: _touchStart,
              onLive: (m) => _live('points', _shapeOf(m)),
              onCommit: (m) => _write('points', (_) => _shapeOf(m), all: false),
              onCancel: _touchCancel,
            ),
          );
          final rows = Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _choiceRow(t, 'resolution', enabled),
              _toggle(t, 'keep_luma', enabled),
              _slider(t, 'strength', enabled),
              _slider(t, 'mix', enabled),
            ],
          );
          if (side) {
            return Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: gradeInset),
                  child: mesh,
                ),
                Expanded(
                  child: Align(
                    alignment: AlignmentDirectional.topStart,
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 420),
                      child: rows,
                    ),
                  ),
                ),
              ],
            );
          }
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Center(child: mesh),
              const SizedBox(height: 6),
              rows,
            ],
          );
        }),
      ],
    );
  }
}
