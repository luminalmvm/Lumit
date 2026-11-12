// The Grade panel's Curves page: the custom curves (luma, red, green, blue)
// with soft clip, and the six hue curves.

part of 'grade_panel_frb.dart';

const List<String> _toneCurves = ['curve_y', 'curve_r', 'curve_g', 'curve_b'];
const List<String> _hueCurves = [
  'hue_hue',
  'hue_sat',
  'hue_lum',
  'lum_sat',
  'sat_sat',
  'sat_lum',
];
const List<String> _curveRows = [
  ..._toneCurves,
  'soft_high',
  'soft_low',
  ..._hueCurves,
];

mixin _GradeCurves on _GradeModel, _GradePrimaries {
  /// Custom (null) or one of [_hueCurves].
  String? _curveMode;
  int _curveChannel = 0;
  final GlobalKey<GradeHueCurveState> _hueCurve = GlobalKey();

  static const double _plotMax = 300;

  /// The soft clip rows' width beside a curve.
  static const double _softWidth = 280;

  /// The row a curve mode control takes, its padding included.
  static const double _modeRow = Segment.height + 12;

  /// The swatches and the picker under a hue curve.
  static const double _swatchRow = 30;

  String _curveModeLabel(String? mode) =>
      mode == null ? l10n.gradeCustom : _label(mode);

  /// One page of curves. In a pane wide enough the custom curve, its soft
  /// clip and a hue curve stand side by side, each as tall as the pane; else
  /// the modes take turns. A pane that stacks its pages sizes them by width.
  Widget _curvesPage(LumitTheme t, bool enabled) => LayoutBuilder(
        builder: (context, box) {
          final room = _room;
          final labels = [for (final id in _toneCurves) _label(id)];
          final column = channelColumnWidth(labels) + channelColumnGap;
          final inner = box.maxWidth - gradeInset * 2;
          if (room != null) {
            final plot = (room.height - 6).clamp(96.0, _plotMax).toDouble();
            final side = plot + column + 16 + _softWidth + 16;
            if (inner >= side + plot * 2) {
              final hue = _curveMode ?? _hueCurves.first;
              return Padding(
                padding: const EdgeInsets.fromLTRB(gradeInset, 6, gradeInset, 0),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _customCurve(t, enabled, labels, plot),
                    const SizedBox(width: 16 - gradeInset),
                    SizedBox(width: _softWidth, child: _softClip(t, enabled)),
                    const SizedBox(width: 16 - gradeInset),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          _modeBar(t, _hueCurves, hue),
                          _hueCurvePane(t, hue, enabled,
                              height: plot - _modeRow - _swatchRow,
                              inset: 0),
                        ],
                      ),
                    ),
                  ],
                ),
              );
            }
          }
          final modes = <String?>[null, ..._hueCurves];
          final mode = _curveMode;
          final tall = room == null ? null : room.height - _modeRow - 6;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: gradeInset),
                child: _modeBar(t, modes, mode),
              ),
              if (mode != null)
                _hueCurvePane(t, mode, enabled,
                    height: tall == null ? null : tall - _swatchRow)
              else
                () {
                  final plot = math
                      .min(inner - column, tall ?? _plotMax)
                      .clamp(48.0, _plotMax)
                      .toDouble();
                  final curve = Padding(
                    padding: const EdgeInsetsDirectional.only(start: gradeInset),
                    child: _customCurve(t, enabled, labels, plot),
                  );
                  // The soft clip beside the curve where it fits, else under.
                  if (inner >= plot + column + 16 + _softWidth) {
                    return Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        curve,
                        const SizedBox(width: 16 - gradeInset),
                        SizedBox(
                            width: _softWidth, child: _softClip(t, enabled)),
                      ],
                    );
                  }
                  return Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Align(
                          alignment: AlignmentDirectional.centerStart,
                          child: curve),
                      const SizedBox(height: 4),
                      _softClip(t, enabled),
                    ],
                  );
                }(),
            ],
          );
        },
      );

  /// The curve modes as a segmented control while their names fit, else a
  /// dropdown.
  Widget _modeBar(LumitTheme t, List<String?> modes, String? mode) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: LayoutBuilder(builder: (context, box) {
          final need = Segment.height +
              modes.fold<double>(
                  0,
                  (sum, m) =>
                      sum +
                      _GradePanelFrbState._textWidth(t, _curveModeLabel(m)) +
                      20);
          if (need > box.maxWidth) {
            return Align(
              alignment: AlignmentDirectional.centerStart,
              child: BareDropdown<String?>(
                key: const ValueKey('grade-curve-menu'),
                value: mode,
                options: modes,
                label: _curveModeLabel,
                onChanged: (m) => setState(() => _curveMode = m),
              ),
            );
          }
          return Align(
            alignment: AlignmentDirectional.centerStart,
            child: Segment(children: [
              for (final m in modes)
                SegmentOption(
                  key: ValueKey<String>('grade-curve-${m ?? 'custom'}'),
                  active: m == mode,
                  onTap: () => setState(() => _curveMode = m),
                  child: Text(_curveModeLabel(m),
                      style: t.small.copyWith(
                          color:
                              m == mode ? t.textPrimary : t.textSecondary)),
                ),
            ]),
          );
        }),
      );

  void _curveLive(String id, List<List<double>> p) {
    if (_touches == 0) _touchStart();
    _live(id, curveValue(p));
  }

  void _curveCommit(String id, List<List<double>> p) {
    if (_touches == 0) _touchStart();
    _write(id, (_) => curveValue(p));
  }

  static String _channelOf(String id) => switch (id) {
        'curve_r' => 'red',
        'curve_g' => 'green',
        'curve_b' => 'blue',
        _ => 'master',
      };

  /// The custom curve [plot] square, with its channel buttons beside it.
  Widget _customCurve(
      LumitTheme t, bool enabled, List<String> labels, double plot) {
    final channel = _curveChannel.clamp(0, _toneCurves.length - 1);
    final id = _toneCurves[channel];
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: [id]),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          IgnorePointer(
            ignoring: !enabled,
            child: Opacity(
              opacity: enabled ? 1 : 0.5,
              child: CurveEditor(
                key: ValueKey<String>('grade-curve-$id'),
                points: _curve(id),
                size: plot,
                line: curveChannelColour(_channelOf(id)),
                rest: _curveRest(id),
                onLive: (p) => _curveLive(id, p),
                onCommit: (p) => _curveCommit(id, p),
              ),
            ),
          ),
          const SizedBox(width: channelColumnGap),
          ChannelButtons(
            labels: labels,
            channel: channel,
            onChannel: (i) => setState(() => _curveChannel = i),
            keyPrefix: 'grade-curves',
            colours: [
              for (final id in _toneCurves) curveChannelColour(_channelOf(id))
            ],
          ),
        ],
      ),
    );
  }

  /// High and low soft clip. Slider rows keep their own inset, so a caller
  /// beside a curve leaves that much less room.
  Widget _softClip(LumitTheme t, bool enabled) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final id in const ['soft_high', 'soft_low'])
            _slider(t, id, enabled, decimals: 0),
        ],
      );

  HueCurveInput _inputOf(String id) => switch (id) {
        'lum_sat' => HueCurveInput.luma,
        'sat_sat' || 'sat_lum' => HueCurveInput.saturation,
        _ => HueCurveInput.hue,
      };

  /// One hue curve with its swatches and picker. [height], given, is the
  /// curve's own height; otherwise it is half its width.
  Widget _hueCurvePane(LumitTheme t, String id, bool enabled,
      {double? height, double inset = gradeInset}) {
    final input = _inputOf(id);
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: [id]),
      child: Padding(
        padding: EdgeInsets.symmetric(horizontal: inset),
        child: LayoutBuilder(builder: (context, box) {
          final h = height == null
              ? (box.maxWidth.clamp(48.0, 640.0) * 0.5)
                  .clamp(90.0, 260.0)
                  .toDouble()
              : height.clamp(60.0, 260.0).toDouble();
          // Long enough to place a point with care, never a ribbon.
          final width = math.min(box.maxWidth, h * 5).clamp(48.0, 960.0)
              .toDouble();
          return SizedBox(
            width: width,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                GradeHueCurve(
                  key: _hueCurve,
                  points: _curve(id),
                  input: input,
                  width: width,
                  height: h,
                  enabled: enabled,
                  keyed: false,
                  rest: _curveRest(id),
                  onLive: (p) => _curveLive(id, p),
                  onCommit: (p) => _curveCommit(id, p),
                ),
                const SizedBox(height: 6),
                Row(
                  children: [
                    if (input == HueCurveInput.hue)
                      Flexible(
                        child: SingleChildScrollView(
                          scrollDirection: Axis.horizontal,
                          child: GradeVectorSwatches(
                            enabled: enabled,
                            onPick: (hue) =>
                                _hueCurve.currentState?.dropThree(hue, 1 / 24),
                          ),
                        ),
                      )
                    else
                      const Spacer(),
                    if (enabled)
                      DropperButton(
                        id: 'grade-hue-pick',
                        tip: l10n.gradePickOnPicture,
                        arm: (ui) => ui.armDropper(DropperArm(
                          id: 'grade-hue-pick',
                          reads: DropperReads.colour,
                          label: l10n.gradePickOnPicture,
                          onPick: (s) => _huePick(id, s),
                        )),
                      ),
                  ],
                ),
              ],
            ),
          );
        }),
      ),
    );
  }


  /// Three points where the picked part of the picture sits along the curve.
  /// The solver reads the node's own input; without it the pick's colour,
  /// in the node's space, says where.
  Future<void> _huePick(String id, DropperSample s) async {
    final shot = _shot;
    final comp = ui.selectedComp;
    final node = _node;
    if (shot == null || comp == null) return;
    (double, double)? found;
    if (_GradeModel._engine.solversReady && node != null) {
      final size = comp.getSize();
      found = await _GradeModel._engine.hueCurvePick(
          comp,
          shot,
          ui.playheadFrame.value,
          node.id,
          GradeCircle(
              Offset(s.xFrac * size.width, s.yFrac * size.height),
              s.region / 2),
          id);
    }
    found ??= _pickedLocally(id, s);
    if (!mounted) return;
    _hueCurve.currentState?.dropThree(found.$1, found.$2);
  }

  /// The sample in the node's grading space, read on the curve's input.
  (double, double) _pickedLocally(String id, DropperSample s) {
    double encode(double x) => switch (_choice('space')) {
          // Linear keeps the value; Log is close enough to video for a place
          // along a curve.
          2 => x,
          _ => x <= 0 ? 0 : math.pow(x, 1 / 2.4).toDouble(),
        };
    final r = encode(s.r), g = encode(s.g), b = encode(s.b);
    return switch (_inputOf(id)) {
      HueCurveInput.hue => (gradeHueOf(r, g, b), 1 / 24),
      HueCurveInput.luma => (
          (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 1.0),
          0.08
        ),
      HueCurveInput.saturation => (
          (math.max(r, math.max(g, b)) - math.min(r, math.min(g, b)))
              .clamp(0.0, 1.0),
          0.08
        ),
    };
  }
}
