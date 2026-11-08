// The graph editor's right-click menus: a key, a speed point, a stretch of
// curve, a tangent handle, a row of the curve list and the empty plot. Every row
// calls what the strip, the keymap and the gestures already call.

part of 'graph_editor_frb.dart';

/// One row: shuts the menu, then runs. [action] is the keymap's, for its chord.
/// Every row keeps the tick column, so the names in a menu share one edge.
Widget _tickRow(
  void Function(Object?) close,
  String label,
  VoidCallback? run, {
  String? action,
  bool ticked = false,
  String? keyName,
}) =>
    MenuRow(
      key: keyName == null ? null : ValueKey<String>(keyName),
      enabled: run != null,
      onPressed: () {
        close(null);
        run?.call();
      },
      child: MenuLabel(label,
          action: action, ticked: ticked, enabled: run != null),
    );

/// The width every menu on the pane opens at, chords included.
const double _menuWidth = 218;

extension _GraphMenus on GraphEditorFrbState {
  /// The keys a menu opened on [id] acts on: the selection when it holds [id],
  /// else [id] alone, which it selects so the menu acts on what was clicked.
  Set<String> _pickFor(String id) {
    if (!widget.selectedKeys.contains(id)) {
      widget.selectedKeys
        ..clear()
        ..add(id);
      widget.onSelectionChanged();
    }
    return widget.selectedKeys;
  }

  ProjectReference? get _project =>
      Provider.of<LumitState>(context, listen: false).project;

  /// The clipboard's two rows for keys, the chords' own road.
  List<Widget> _keyClipboardRows(void Function(Object?) close) => [
        _tickRow(close, l10n.menuCopy, widget.onCopyKeys, action: 'edit.copy'),
        _tickRow(close, l10n.menuPaste,
            graphKeyClipboard.isEmpty ? null : widget.onPasteKeys,
            action: 'edit.paste'),
      ];

  // --- a key ---------------------------------------------------------------

  /// The key's menu. [side] is the speed-lens dot it was opened on, or null
  /// for the value lens's one key.
  Future<void> _showKeyMenu(GraphChannel channel, int index, Offset position,
      {bool? side}) async {
    if (isEnvelope(channel)) {
      return _showSpeedPointMenu(channel, index, position);
    }
    final keys = _pickFor('${channel.id}#$index');
    await showMenuAt<Object>(
      context: context,
      position: position,
      width: keyMenuWidth,
      rows: (close) => channelKeyMenuRows(
        context,
        close: () => close(null),
        keyPrefix: 'graph-key-menu',
        channels: () => _channels,
        picked: (_) => keys,
        count: keys.length,
        onChanged: widget.onChanged,
        project: _project,
        fps: widget.fps,
        fpsNum: widget.fpsNum,
        fpsDen: widget.fpsDen,
        onEase: widget.onOpenEase == null
            ? null
            : () => widget.onOpenEase!(position),
        onSpeed: () => _openKeySpeed(channel, index, side: side),
        onCopy: widget.onCopyKeys,
        onPaste: widget.onPasteKeys,
        onDelete: _deleteSelection,
      ),
    );
  }

  // --- a speed point -------------------------------------------------------

  /// A speed point's menu: type its speed, put it back to 100%, choose the
  /// curve the speed takes to the next point, or delete it. The easing rows
  /// a key has would bend the envelope out of the shape its points describe.
  Future<void> _showSpeedPointMenu(
      GraphChannel channel, int index, Offset position) async {
    final picked = _pickFor('${channel.id}#$index');
    final keys = channel.keys;
    // The points picked on this curve, which is the one being written.
    final points = [
      for (var i = 0; i < keys.length; i++)
        if (picked.contains('${channel.id}#$i')) i,
    ];
    final current =
        index + 1 < keys.length ? speedCurveOf(keys[index].interpOut) : null;
    void write(List<BridgeKeyframe> next) {
      commitChannelEdits({channel: BridgeScalar.keyframed(next)});
      widget.onChanged();
    }

    await showMenuAt<Object>(
      context: context,
      position: position,
      width: _menuWidth,
      rows: (close) => [
        _tickRow(close, l10n.menuSetSpeed,
            () => _openKeyFields(channel, index, position),
            keyName: 'graph-key-menu-speed'),
        _tickRow(close, l10n.speedPointReset, () {
          var next = keys;
          for (final i in points) {
            next = setEnvelopeSpeed(next, i, 100);
          }
          write(next);
        }),
        if (current != null) ...[
          const MenuDivider(),
          ..._curveRows(close, channel, index, current),
        ],
        const MenuDivider(),
        ..._keyClipboardRows(close),
        const MenuDivider(),
        _tickRow(close, l10n.speedPointDelete,
            keys.length - points.length >= 2 ? _deleteSelection : null,
            action: 'edit.delete.selection'),
      ],
    );
  }

  /// The six curves a speed span can take, the one in force ticked.
  List<Widget> _curveRows(void Function(Object?) close, GraphChannel channel,
          int span, SpeedCurve current) =>
      [
        for (final (curve, name) in [
          (SpeedCurve.linear, l10n.speedCurveLinear),
          (SpeedCurve.fast, l10n.speedCurveFast),
          (SpeedCurve.slow, l10n.speedCurveSlow),
          (SpeedCurve.smooth, l10n.speedCurveSmooth),
          (SpeedCurve.sharp, l10n.speedCurveSharp),
          (SpeedCurve.hold, l10n.speedCurveHold),
        ])
          _tickRow(close, name, () {
            commitChannelEdits({
              channel: BridgeScalar.keyframed(
                  setEnvelopeCurve(channel.keys, span, curve)),
            });
            widget.onChanged();
          },
              ticked: curve == current,
              keyName: 'graph-speed-curve-${curve.name}'),
      ];

  // --- a stretch of curve --------------------------------------------------

  /// Which span of [channel] [local] stands over: the key it starts at, or
  /// null off either end.
  int? _spanAt(GraphChannel channel, Offset local) {
    final at = _secondsOfX(local.dx);
    final keys = channel.keys;
    final i = keys.lastIndexWhere((k) => rationalSeconds(k.time) <= at);
    return i < 0 || i + 1 >= keys.length ? null : i;
  }

  /// The menu on the curve between keys. Falls through to the plot's when the
  /// pointer is on neither.
  void _showPaneMenu(TapDownDetails d, (double, double) range, double height) {
    final channel = _curveAt(d.localPosition, range, height, within: 6);
    if (channel == null) {
      _showPlotMenu(d.localPosition, d.globalPosition, range, height);
      return;
    }
    if (isEnvelope(channel)) {
      _showEnvelopeLineMenu(
          channel, d.localPosition, d.globalPosition, range, height);
    } else {
      _showCurveMenu(channel, d.localPosition, d.globalPosition, range, height);
    }
  }

  void _showCurveMenu(GraphChannel channel, Offset local, Offset global,
      (double, double) range, double height) {
    final span = _spanAt(channel, local);
    void shape(BridgeSideInterp side) {
      applyInterpToSpan(channel, span!, side);
      widget.onChanged();
    }

    final shapes = span != null && !channel.isStatic;
    showMenuAt<Object>(
      context: context,
      position: global,
      width: _menuWidth,
      rows: (close) => [
        _tickRow(
            close,
            l10n.graphAddKeyHere,
            channel.isStatic || channel.isPath
                ? null
                : () => _addKeyAt(local, range, height)),
        const MenuDivider(),
        _tickRow(close, l10n.easeLinear,
            shapes ? () => shape(const BridgeSideInterp.linear()) : null),
        _tickRow(close, l10n.easeEasy, shapes ? () => shape(easyEase) : null),
        _tickRow(close, l10n.easeHold,
            shapes ? () => shape(const BridgeSideInterp.hold()) : null),
        const MenuDivider(),
        ..._curveListRows(close, channel),
      ],
    );
  }

  void _showEnvelopeLineMenu(GraphChannel channel, Offset local, Offset global,
      (double, double) range, double height) {
    final keys = channel.keys;
    final span = _spanAt(channel, local);
    void write(List<BridgeKeyframe> next) {
      commitChannelEdits({channel: BridgeScalar.keyframed(next)});
      widget.onChanged();
    }

    showMenuAt<Object>(
      context: context,
      position: global,
      width: _menuWidth,
      rows: (close) => [
        _tickRow(
            close, l10n.speedPointAdd, () => _addKeyAt(local, range, height)),
        if (span != null) ...[
          const MenuDivider(),
          ..._curveRows(
              close, channel, span, speedCurveOf(keys[span].interpOut)),
        ],
        const MenuDivider(),
        _tickRow(close, l10n.speedThin, () => write(thinEnvelope(keys))),
        _tickRow(close, l10n.clipResetSpeed, () => write(resetEnvelope(keys))),
        const MenuDivider(),
        ..._curveListRows(close, channel, reset: false),
      ],
    );
  }

  // --- a tangent handle ----------------------------------------------------

  void _showHandleMenu(
      GraphChannel channel, int index, bool isOut, Offset position) {
    final keys = _pickFor('${channel.id}#$index');
    final split = anyHandlesSplit(channels: _channels, selectedKeys: keys);
    showMenuAt<Object>(
      context: context,
      position: position,
      width: _menuWidth,
      rows: (close) => [
        _tickRow(close, l10n.graphFlattenHandle, () {
          _forgetHandleLengths(channel, index);
          flattenHandlesOnSelection(
              channels: _channels,
              selectedKeys: keys,
              // Joined, the pair lies flat as one line. Split, this side alone.
              isOut: split ? isOut : null);
          widget.onChanged();
        }),
        _tickRow(
            close,
            l10n.keyMenuJoinHandles,
            split
                ? () {
                    _forgetHandleLengths(channel, index);
                    joinHandlesOnSelection(
                        channels: _channels, selectedKeys: keys);
                    widget.onChanged();
                  }
                : null),
        const MenuDivider(),
        _tickRow(close, l10n.easeLinear, () {
          applyInterpToSelection(
              channels: _channels,
              selectedKeys: keys,
              side: const BridgeSideInterp.linear(),
              inSide: !isOut,
              outSide: isOut);
          widget.onChanged();
        }),
        _tickRow(close, l10n.menuKeyframeSpeed,
            () => _openKeySpeed(channel, index, side: isOut),
            action: 'keyframe.speed'),
        // A mode of the pane, not something done to this handle, so it stands
        // apart from the rows that are.
        const MenuDivider(),
        _tickRow(close, l10n.graphBreakHandles, widget.onToggleBreakHandles,
            ticked: widget.breakHandles),
      ],
    );
  }

  // --- a curve in the list -------------------------------------------------

  void _setHidden(Set<String> ids) {
    widget.selectedKeys.removeWhere(
        (key) => ids.contains(key.substring(0, key.lastIndexOf('#'))));
    hiddenCurves.value = ids;
    widget.onSelectionChanged();
  }

  void _solo(GraphChannel channel) => _setHidden({
        for (final c in widget.channels)
          if (c.id != channel.id) c.id,
      });

  void _selectKeysOf(GraphChannel channel) {
    if (_dashed(channel)) widget.onPick?.call(channel.path);
    widget.selectedKeys
      ..clear()
      ..addAll([
        for (var i = 0; i < channel.keys.length; i++) '${channel.id}#$i',
      ]);
    widget.onSelectionChanged();
  }

  /// What [channel] reads with nothing done to it, or null when that is not
  /// one number.
  double? _defaultOf(GraphChannel channel) {
    if (channel.volume) return 0;
    if (channel.param case final param?) {
      return switch (defaultEffectValue(param.kind)) {
        BridgeEffectValue_Float(field0: BridgeScalar_Static(:final field0)) =>
          field0,
        _ => null,
      };
    }
    if (channel.prop case final prop?) {
      final size = widget.comp.getSettings();
      return transformDefault(
          prop, size.width.toDouble(), size.height.toDouble());
    }
    return null;
  }

  /// A curve's reset: a Retime goes back to 100%, anything else with a default
  /// back to that, keys gone. Null when it has neither.
  VoidCallback? _resetOf(GraphChannel channel) {
    if (channel.retime && channel.keys.length >= 2) {
      return () {
        commitChannelEdits(
            {channel: BridgeScalar.keyframed(resetEnvelope(channel.keys))});
        widget.onChanged();
      };
    }
    final fresh = _defaultOf(channel);
    if (fresh == null) return null;
    return () {
      commitChannelEdits({channel: BridgeScalar.static_(fresh)});
      widget.onChanged();
    };
  }

  /// A small box holding the layer's name, written on Enter.
  void _renameLayer(GraphChannel channel, Offset position) {
    final field = TextEditingController(text: channel.entry.info.name);
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => FloatSurface(
        width: 200,
        child: HouseTextField(
          key: const ValueKey('graph-rename-layer'),
          controller: field,
          autofocus: true,
          onSubmitted: (name) {
            close(null);
            if (name.trim().isEmpty) return;
            try {
              channel.entry.layer.rename(name: name.trim());
            } catch (_) {
              return;
            }
            widget.onChanged();
          },
        ),
      ),
    ).whenComplete(field.dispose);
  }

  /// The rows a curve answers wherever it is clicked, on the plot or in the
  /// list.
  List<Widget> _curveListRows(
      void Function(Object?) close, GraphChannel channel,
      {bool reset = true}) {
    final hidden = hiddenCurves.value;
    final several = widget.channels.length > 1;
    final soloed = several &&
        widget.channels.every((c) => c.id == channel.id
            ? !hidden.contains(c.id)
            : hidden.contains(c.id));
    return [
      if (several) ...[
        _tickRow(close, l10n.switchSolo,
            () => soloed ? _setHidden(const {}) : _solo(channel),
            ticked: soloed),
        _tickRow(
            close,
            l10n.graphHideCurve,
            () => _setHidden(hidden.contains(channel.id)
                ? ({...hidden}..remove(channel.id))
                : {...hidden, channel.id}),
            ticked: hidden.contains(channel.id)),
        const MenuDivider(),
      ],
      _tickRow(close, l10n.graphSelectKeys,
          channel.keys.isEmpty ? null : () => _selectKeysOf(channel)),
      _tickRow(
          close,
          l10n.menuCopy,
          channel.keys.isEmpty
              ? null
              : () {
                  _selectKeysOf(channel);
                  widget.onCopyKeys?.call();
                },
          action: 'edit.copy'),
      _tickRow(
          close,
          l10n.menuPaste,
          graphKeyClipboard.isEmpty
              ? null
              : () {
                  if (_dashed(channel)) widget.onPick?.call(channel.path);
                  widget.onPasteKeys?.call();
                },
          action: 'edit.paste'),
      if (reset) ...[
        const MenuDivider(),
        _tickRow(close, l10n.reset, _resetOf(channel)),
      ],
    ];
  }

  void _showCurveListMenu(GraphChannel channel, Offset position) {
    showMenuAt<Object>(
      context: context,
      position: position,
      width: _menuWidth,
      rows: (close) => [
        ..._curveListRows(close, channel),
        _tickRow(close, l10n.graphRenameLayer,
            () => _renameLayer(channel, position)),
      ],
    );
  }

  // --- the empty plot ------------------------------------------------------

  void _showPlotMenu(
      Offset local, Offset global, (double, double) range, double height) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final keyed = _channels.where((c) => !_dashed(c) && !c.isStatic).toList();
    final hidden = _anyHidden;
    final allKeys = {
      for (final c in _channels)
        for (var i = 0; i < c.keys.length; i++) '${c.id}#$i',
    };
    // On the speed graph, the Retime row's own two first.
    final frame = ui.playheadFrame.value;
    final retimed = widget.lens == GraphLens.speed
        ? retimeTargets(ui)
        : const <BridgeLayerEntry>[];
    showMenuAt<Object>(
      context: context,
      position: global,
      width: _menuWidth,
      rows: (close) => [
        if (retimed.isNotEmpty) ...[
          _tickRow(
              close,
              l10n.retimeFreeze,
              retimed.any((e) => canFreezeAt(e, frame))
                  ? () => freezeAtPlayhead(context, retimed)
                  : null,
              action: 'layer.retime.hold',
              keyName: 'graph-plot-freeze'),
          _tickRow(
              close,
              l10n.retimeRamp,
              retimed.any(
                      (e) => frame >= e.info.inFrame && frame < e.info.outFrame)
                  ? () => askToRamp(context, retimed, const [], global)
                  : null,
              action: 'layer.retime.ramp',
              keyName: 'graph-plot-ramp'),
          const MenuDivider(),
        ],
        _tickRow(
            close,
            l10n.graphAddKeyAtPlayhead,
            keyed.isEmpty
                ? null
                : () => asOneUndoStep(_project, () {
                      for (final channel in keyed) {
                        _plantAtFrame(
                            channel, ui.playheadFrame.value.toDouble());
                      }
                    }),
            keyName: 'graph-plot-add-key'),
        _tickRow(close, l10n.menuPaste,
            graphKeyClipboard.isEmpty ? null : widget.onPasteKeys,
            action: 'edit.paste'),
        _tickRow(
            close,
            l10n.graphSelectAllKeys,
            allKeys.isEmpty
                ? null
                : () {
                    widget.selectedKeys
                      ..clear()
                      ..addAll(allKeys);
                    widget.onSelectionChanged();
                  }),
        const MenuDivider(),
        // The node canvases' order and keys: F the picked, Shift+F the lot.
        _tickRow(close, l10n.graphFrameSelected,
            widget.selectedKeys.isEmpty ? null : widget.onFrame,
            action: 'graph.fit'),
        _tickRow(close, l10n.graphFrameAll, widget.onFrameAll,
            keyName: 'graph-plot-frame-all', action: 'graph.frame.all'),
        _tickRow(close, l10n.graphAutoFit, widget.onToggleAutoFit,
            ticked: widget.autoFit),
        const MenuDivider(),
        for (final (lens, name) in [
          (GraphLens.value, l10n.tipValueGraph),
          (GraphLens.speed, l10n.tipSpeedGraph),
          (GraphLens.acceleration, l10n.tipAccelerationGraph),
        ])
          _tickRow(close, name,
              widget.onLens == null ? null : () => widget.onLens!(lens),
              ticked: widget.lens == lens),
        const MenuDivider(),
        for (final (scope, name) in [
          (GraphScope.selected, l10n.tipGraphScopeSelected),
          (GraphScope.layer, l10n.tipGraphScopeLayer),
          (GraphScope.comp, l10n.tipGraphScopeComp),
        ])
          _tickRow(close, name,
              widget.onScope == null ? null : () => widget.onScope!(scope),
              ticked: widget.scope == scope),
        if (hidden)
          _tickRow(close, l10n.graphShowAllCurves, () => _setHidden(const {})),
        const MenuDivider(),
        _tickRow(close, l10n.easingSnap, widget.onToggleMagnet,
            ticked: widget.magnet),
      ],
    );
  }
}
