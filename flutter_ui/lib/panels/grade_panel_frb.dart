// The Grade panel: the current node of the current shot, as a colourist's
// palettes. Wheels, Adjust, Curves, Qualifier, Windows, Match and Look.
//
// The shot is the clip picked in the Cut timeline, else the selected layer,
// and its grade is the Colour effects in its own stack. A node is one of
// those effects, usually a Correction. Nothing is added until a control is
// touched; the first touch adds Correction 1 in its place, and that and the
// value are one undo step. Effect controls, the Timeline and the Graph panel
// still show the same effects, which is where their keyframes are set; a
// keyed value here takes a key at the playhead, as a keyed row does.

import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/grade_groups.dart';
import 'package:lumit_flutter/src/rust/api/grade_lut.dart';
import 'package:lumit_flutter/src/rust/api/grade_shared.dart';
import 'package:lumit_flutter/src/rust/api/grade_shots.dart';
import 'package:lumit_flutter/src/rust/api/grade_versions.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/theme_name_dialog.dart';
import '../state/comp_time.dart';
import '../state/dock.dart';
import '../state/dropper.dart';
import '../state/template_controls.dart' show ControlRow;
import '../state/file_dialogs.dart';
import '../state/grade_engine.dart';
import '../state/grade_target.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart' show hsvToRgb;
import '../widgets/controls.dart';
import '../widgets/curve_editor.dart';
import '../widgets/edge_fade.dart';
import '../widgets/time_readout.dart' show monoSlotWidth;
import 'control_rows.dart' show paramControlRow;
import 'effect_controls_panel_frb.dart' show curveChannelColour;
import 'effect_param_row_frb.dart'
    show
        DropperButton,
        EffectParamRowFrb,
        EffectStackEditor,
        curvePointsOf,
        curveValue,
        defaultEffectValue,
        newExpressionLanguage,
        scalarValueAt;
import 'grade_actions.dart';
import 'grade_bars.dart';
import 'grade_clipboard.dart';
import 'grade_controls.dart';
import 'grade_hue_curve.dart';
import 'grade_looks.dart';
import 'grade_range_bar.dart';
import 'grade_warper.dart';
import 'grade_wheel.dart';
import 'keyframe_controls_frb.dart'
    show KeyframeControlsFrb, scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep, effectPath;
import 'param_row_menu.dart';
import 'timeline_extras_frb.dart' show Segment, SegmentOption;
import 'viewer_grade_windows.dart' show gradeWindowStart;

part 'grade_model.dart';
part 'grade_primaries.dart';
part 'grade_hdr.dart';
part 'grade_curves.dart';
part 'grade_qualifier.dart';
part 'grade_windows.dart';
part 'grade_match.dart';
part 'grade_generic.dart';

class GradePanelFrb extends StatefulWidget {
  const GradePanelFrb({super.key});

  @override
  State<GradePanelFrb> createState() => _GradePanelFrbState();
}

class _GradePanelFrbState extends State<GradePanelFrb>
    with
        _GradeModel,
        _GradePrimaries,
        _GradeHdr,
        _GradeCurves,
        _GradeQualifier,
        _GradeWindows,
        _GradeMatch,
        _GradeGeneric {
  StreamSubscription<void>? _changes;

  /// From this width Adjust stands beside Wheels and Look beside Match,
  /// instead of on pages of their own.
  static const double _wideAt = 1000;

  /// Below this width and at twice as tall, the pages stack as twirls.
  static const double _stackBelow = 520;

  /// Which twirls are open while the pages stack.
  final Map<GradePage, bool> _open = {GradePage.wheels: true};

  @override
  void initState() {
    super.initState();
    _changes = app.onChange.listen((_) {
      if (_restage != null) scheduleMicrotask(() => _restage?.call());
    });
  }

  @override
  void dispose() {
    _changes?.cancel();
    _trackStop(quiet: true);
    _editor.clear();
    while (_touches > 0) {
      _touches--;
      _touched?.endUndoGroup();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ui = this.ui;
    final grade = ui.grade;
    return ListenableBuilder(
      listenable: Listenable.merge([
        ui.model,
        grade.shot,
        grade.node,
        grade.page,
        grade.wheelMode,
        grade.scope,
        grade.window,
        grade.matte,
        grade.pick,
        grade.reference,
        ui.selectedLayers,
        ui.cutClips,
        ui.playheadFrame,
      ]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final shot = _shot;
        final enabled =
            shot != null && ui.selectedComp != null && _gradeable(shot);
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(t, enabled),
            Expanded(
              child: GradeMenu(
                rows: _groundMenu,
                child: Container(
                  color: t.surface0,
                  child: LayoutBuilder(builder: (context, box) {
                    if (enabled && !_onCorrection) {
                      return _scroll(_generic(t));
                    }
                    final stacked = box.maxWidth < _stackBelow &&
                        box.maxHeight > box.maxWidth * 2;
                    _room = stacked
                        ? null
                        : Size(box.maxWidth,
                            box.maxHeight - Segment.height - 16);
                    if (stacked) return _scroll(_stacked(t, enabled));
                    final wide = box.maxWidth >= _wideAt;
                    var page = grade.page.value;
                    if (_besideWheels(wide) && page == GradePage.adjust) {
                      page = GradePage.wheels;
                    }
                    if (wide && page == GradePage.look) {
                      page = GradePage.match;
                    }
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _pageBar(t, page, enabled, wide: wide),
                        Expanded(
                          child: _scroll(
                            KeyedSubtree(
                              key: ValueKey<GradePage>(page),
                              child: _page(t, page, enabled, wide: wide),
                            ),
                          ),
                        ),
                      ],
                    );
                  }),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  Widget _scroll(Widget child) => EdgeFade(
        axis: Axis.vertical,
        child: SingleChildScrollView(
          primary: false,
          padding: const EdgeInsets.only(bottom: 8),
          child: child,
        ),
      );

  static String _pageLabel(GradePage p) => switch (p) {
        GradePage.wheels => l10n.gradeWheels,
        GradePage.adjust => l10n.gradeAdjust,
        GradePage.curves => l10n.fxCurves,
        GradePage.qualifier => l10n.gradeQualifier,
        GradePage.windows => l10n.gradeWindows,
        GradePage.match => l10n.gradeMatch,
        GradePage.look => l10n.fxLook,
      };

  /// The rows a page's reset puts back, none for a page with nothing to.
  List<String> _pageRows(GradePage p) => switch (p) {
        GradePage.wheels => [..._wheelRows, ..._stripRows, ..._hdrRows],
        GradePage.adjust => [..._adjustRows, ..._mixerRows],
        GradePage.curves => _curveRows,
        GradePage.qualifier => [
            for (final id in _schema.keys)
              if (id.startsWith('qual_')) id
          ],
        GradePage.windows => [
            for (final id in _schema.keys)
              if (id.startsWith('win')) id
          ],
        GradePage.match || GradePage.look => const [],
      };

  /// Whether Adjust stands beside the wheels: in a wide pane, unless HDR
  /// wants the whole width for its seven cells.
  bool _besideWheels(bool wide) => wide && _wheelMode != GradeWheelMode.hdr;

  Widget _page(LumitTheme t, GradePage p, bool enabled, {bool wide = false}) =>
      switch (p) {
        // The wheels keep their own width and Adjust has the rest.
        GradePage.wheels => _besideWheels(wide)
            ? Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SizedBox(
                      width: _wheelsWidth(t), child: _wheelsPage(t, enabled)),
                  const SizedBox(width: 12),
                  Expanded(child: _adjustPage(t, enabled)),
                ],
              )
            : _wheelsPage(t, enabled),
        GradePage.adjust => _adjustPage(t, enabled),
        GradePage.curves => _curvesPage(t, enabled),
        GradePage.qualifier => _qualifierPage(t, enabled),
        GradePage.windows => _windowsPage(t, enabled),
        GradePage.match => wide
            ? Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (final (i, label, page) in [
                    (0, l10n.gradeMatch, _matchPage(t, enabled)),
                    (1, l10n.fxLook, _lookPage(t, enabled)),
                  ]) ...[
                    if (i > 0) const SizedBox(width: 12),
                    SizedBox(
                      width: _paletteWidth,
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          GradeHeading(
                              label: label, changed: false, onReset: null),
                          page,
                        ],
                      ),
                    ),
                  ],
                ],
              )
            : _matchPage(t, enabled),
        GradePage.look => _lookPage(t, enabled),
      };

  /// How wide the Match or the Look palette is beside the other.
  static const double _paletteWidth = 440;

  /// The page control and the page's reset. The pages as a segmented
  /// control while their names fit, else a dropdown.
  Widget _pageBar(LumitTheme t, GradePage page, bool enabled,
      {required bool wide}) {
    final pages = [
      for (final p in GradePage.values)
        if (!(_besideWheels(wide) && p == GradePage.adjust) &&
            !(wide && p == GradePage.look))
          p
    ];
    final rows = _pageRows(page);
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: rows),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(gradeInset, 6, 4, 2),
        child: SizedBox(
          height: Segment.height,
          child: Row(
            children: [
              Expanded(
                child: LayoutBuilder(builder: (context, box) {
                  final need = Segment.height +
                      pages.fold<double>(
                          0, (sum, p) => sum + _textWidth(t, _pageLabel(p)) + 27);
                  if (need > box.maxWidth) {
                    return Align(
                      alignment: AlignmentDirectional.centerStart,
                      child: BareDropdown<GradePage>(
                        key: const ValueKey('grade-page-menu'),
                        value: page,
                        options: pages,
                        label: _pageLabel,
                        onChanged: (p) => ui.grade.page.value = p,
                      ),
                    );
                  }
                  return Align(
                    alignment: AlignmentDirectional.centerStart,
                    child: Segment(children: [
                      for (final p in pages)
                        SegmentOption(
                          key: ValueKey<String>('grade-page-${p.name}'),
                          active: p == page,
                          onTap: () => ui.grade.page.value = p,
                          child: gradeDotted(
                              t,
                              Text(_pageLabel(p),
                                  style: t.small.copyWith(
                                      color: p == page
                                          ? t.textPrimary
                                          : t.textSecondary)),
                              enabled &&
                                  _anyChanged([
                                    ..._pageRows(p),
                                    if (_besideWheels(wide) &&
                                        p == GradePage.wheels)
                                      ..._pageRows(GradePage.adjust),
                                  ])),
                        ),
                    ]),
                  );
                }),
              ),
              if (page == GradePage.qualifier) _highlightButton(enabled),
              if (enabled && _anyChanged(rows))
                GradeIconButton(
                  key: const ValueKey('grade-page-reset'),
                  glyph: LumitIcons.reset,
                  tip: l10n.gradeResetPage,
                  onPressed: () => _reset(rows),
                ),
            ],
          ),
        ),
      ),
    );
  }

  static double _textWidth(LumitTheme t, String text) {
    final painter = TextPainter(
      text: TextSpan(text: text, style: t.small),
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout();
    return painter.width;
  }

  /// Every page as a twirl, one under another, for a tall narrow pane.
  Widget _stacked(LumitTheme t, bool enabled) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final p in GradePage.values) ...[
            GradeMenu(
              rows: (close) => _menuFor(close, rows: _pageRows(p)),
              child: GestureDetector(
                key: ValueKey<String>('grade-twirl-${p.name}'),
                behavior: HitTestBehavior.opaque,
                onTap: () =>
                    setState(() => _open[p] = !(_open[p] ?? false)),
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(4, 8, 4, 2),
                  child: SizedBox(
                    height: 18,
                    child: Row(
                      children: [
                        TwirlTurn(
                          open: _open[p] ?? false,
                          child: LumitIcon(
                              (_open[p] ?? false)
                                  ? LumitIcons.collapse
                                  : LumitIcons.expand,
                              size: 12,
                              colour: t.textMuted),
                        ),
                        const SizedBox(width: 2),
                        Expanded(
                          child: Text(_pageLabel(p),
                              style: t.kicker,
                              maxLines: 1,
                              overflow: TextOverflow.clip),
                        ),
                        if (p == GradePage.qualifier) _highlightButton(enabled),
                        if (enabled && _anyChanged(_pageRows(p)))
                          GradeIconButton(
                            key: ValueKey<String>('grade-reset-${p.name}'),
                            glyph: LumitIcons.reset,
                            tip: l10n.gradeResetPage,
                            onPressed: () => _reset(_pageRows(p)),
                          ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
            if (_open[p] ?? false) _page(t, p, enabled),
          ],
        ],
      );

  // --- The header --------------------------------------------------------------

  Widget _header(LumitTheme t, bool enabled) {
    final shot = _shot;
    final targets = enabled ? _targets.length : 0;
    final scope = enabled ? gradeScopeOf(ui) : null;
    final name = !enabled
        ? ''
        : scope != null
            ? (scope.part == BridgeGradePart.pre
                ? l10n.gradeScopeGroupPre
                : l10n.gradeScopeGroupPost)
        : targets > 1
            ? (shot!.clip == null
                ? l10n.gradeLayerCount(targets)
                : l10n.gradeClipCount(targets))
            : _shotName(shot!);
    return GradeMenu(
      rows: _groundMenu,
      child: Container(
        height: 26,
        color: t.surface1,
        padding: const EdgeInsets.only(left: gradeInset, right: 4),
        child: Row(
          children: [
            Expanded(
              child: Text(
                name,
                style: t.small.copyWith(color: t.textMuted),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            if (enabled) Flexible(flex: 2, child: _chips(t)),
            // A group's grade has no versions of its own.
            if (enabled && scope == null) _versionButton(t),
            if (enabled && _anyPresent)
              Listener(
                onPointerDown: (_) => _beforeDown(),
                onPointerUp: (_) => _beforeUp(),
                onPointerCancel: (_) => _beforeUp(),
                child: GradeIconButton(
                  key: const ValueKey('grade-before'),
                  // The effect bypass glyph: held, the grade is bypassed. The
                  // layer's eye one panel down means hide, which this is not.
                  glyph: LumitIcons.bypass,
                  tip: l10n.gradeBefore,
                  active: _before,
                  onPressed: () {},
                ),
              ),
            if (enabled && _anyPresent)
              GradeIconButton(
                key: const ValueKey('grade-reset-all'),
                glyph: LumitIcons.reset,
                tip: l10n.gradeResetAll,
                onPressed: _resetGrade,
              ),
          ],
        ),
      ),
    );
  }

  /// One chip a node, numbered in stack order, then a plus. The current node
  /// wears the accent; a bypassed one is dimmed. A right-click offers what the
  /// node's own menu does.
  Widget _chips(LumitTheme t) {
    final nodes = gradeNumberedOf(_stack);
    final current = _sharedRef ?? _node;
    Widget chip(String key, String label, bool lit, bool off, VoidCallback onTap,
            {String? tip}) =>
        Padding(
          padding: const EdgeInsets.only(left: 2),
          child: LumitTooltip(
            message: tip ?? '',
            child: GestureDetector(
              key: ValueKey<String>(key),
              behavior: HitTestBehavior.opaque,
              onTap: onTap,
              child: HoverMark(
                builder: (context, over) => Container(
                  constraints: const BoxConstraints(minWidth: 18),
                  height: 18,
                  padding: const EdgeInsets.symmetric(horizontal: 4),
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: Color.lerp(t.surface1, t.surface2, over),
                    borderRadius: BorderRadius.circular(t.tokens.actionRadius),
                    border: Border.all(
                        color: lit ? t.accent : t.hairline, width: lit ? 1.5 : 1),
                  ),
                  child: Text(label,
                      style: t.small.copyWith(
                          color: off
                              ? t.textDisabled
                              : lit
                                  ? t.textPrimary
                                  : t.textSecondary)),
                ),
              ),
            ),
          ),
        );
    return Align(
      alignment: AlignmentDirectional.centerEnd,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        reverse: true,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (var i = 0; i < nodes.length; i++)
              GradeMenu(
                rows: (close) {
                  ui.grade.setNode(nodes[i].id);
                  return _groundMenu(close);
                },
                child: chip(
                  'grade-node-${i + 1}',
                  '${i + 1}',
                  nodes[i].id == current?.id,
                  !nodes[i].enabled,
                  () => ui.grade.setNode(nodes[i].id),
                  tip: sharedNodeOf(ui, nodes[i].id)?.name ??
                      nodes[i].customName ??
                      engineLabel(_effectLabel(nodes[i].name)),
                ),
              ),
            chip('grade-node-add', '+', false, false,
                () => _addNode(GradeNodeKind.serial),
                tip: l10n.gradeAddSerial),
          ],
        ),
      ),
    );
  }

  static final Map<String, String> _effectLabels = {
    for (final e in listEffects()) e.name: e.label,
  };
  static String _effectLabel(String name) => _effectLabels[name] ?? name;

  // --- Versions ----------------------------------------------------------------

  /// The shot's versions, read once per document revision.
  (int?, String, List<BridgeGradeVersion>)? _versionsRead;

  List<BridgeGradeVersion> get _versions {
    final shot = _shot;
    if (shot == null) return const [];
    final revision = ui.model.revision;
    final held = _versionsRead;
    if (held != null && held.$1 == revision && held.$2 == shot.key) {
      return held.$3;
    }
    List<BridgeGradeVersion> read;
    try {
      read = shot.layer.getGradeVersions(clip: shot.clip);
    } catch (_) {
      read = const [];
    }
    _versionsRead = (revision, shot.key, read);
    return read;
  }

  Widget _versionButton(LumitTheme t) {
    final versions = _versions;
    final live = versions.where((v) => v.live).firstOrNull;
    return Flexible(
      child: Padding(
        padding: const EdgeInsets.only(left: 6),
        child: Builder(builder: (context) {
          return HouseButton(
            key: const ValueKey('grade-version'),
            frameless: true,
            small: true,
            onPressed: () {
              final box = context.findRenderObject()! as RenderBox;
              GradeMenu.show(
                  context,
                  box.localToGlobal(Offset(0, box.size.height + 2)),
                  _versionRows);
            },
            child: Text(
              live?.name ?? l10n.gradeVersionFirst,
              style: t.small.copyWith(color: t.textSecondary),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          );
        }),
      ),
    );
  }

  List<Widget> _versionRows(VoidCallback close) {
    final shot = _shot;
    if (shot == null) return const [];
    final versions = _versions;
    final live = versions.where((v) => v.live).firstOrNull;
    void run(void Function() edit) {
      try {
        edit();
      } catch (_) {
        // The shot went, or refused; the document is as it was.
      }
      _tell();
    }

    return menuGroups([
      [
        for (final v in versions)
          _row('grade-version-${v.id}', v.name, close,
              () => run(() => shot.layer
                  .switchGradeVersion(clip: shot.clip, version: v.id)),
              ticked: v.live),
      ],
      [
        _row('grade-version-new', l10n.gradeVersionNew, close,
            () => run(() => shot.layer.addGradeVersion(clip: shot.clip, copy: false))),
        _row('grade-version-copy', l10n.gradeVersionDuplicate, close,
            () => run(() => shot.layer.addGradeVersion(clip: shot.clip, copy: true))),
        if (live != null)
          _row('grade-version-rename', l10n.renameEllipsis, close,
              () => _renameVersion(shot, live)),
        if (live != null)
          _row('grade-version-delete', l10n.gradeVersionDelete, close,
              () => run(() => shot.layer
                  .deleteGradeVersion(clip: shot.clip, version: live.id))),
      ],
      [
        _row('grade-version-grid', l10n.gradeGridVersions, close,
            () => toggleGradeGrid(ui, GradeGrid.versions),
            enabled: versions.length > 1,
            ticked: ui.grade.grid.value == GradeGrid.versions),
      ],
    ]);
  }

  Future<void> _renameVersion(GradeShot shot, BridgeGradeVersion v) async {
    final name = await askThemeName(context,
        title: l10n.gradeVersionRenameTitle,
        suggested: v.name,
        confirm: l10n.rename,
        label: l10n.name);
    if (name == null || name.trim().isEmpty || !mounted) return;
    try {
      shot.layer
          .renameGradeVersion(clip: shot.clip, version: v.id, name: name.trim());
    } catch (_) {
      return;
    }
    _tell();
  }
}
