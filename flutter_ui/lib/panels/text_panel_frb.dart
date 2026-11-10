// The Text panel: the font, size, spacing and outline of a text layer's
// letters.
//
// One style for the whole layer. The controls a title is usually set with are
// always shown, and More opens the rest: scale, baseline shift, the faux
// styles, capitals and ligatures.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/font_axes.dart';
import '../state/text_documents.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'text_target.dart';

class TextPanelFrb extends StatefulWidget {
  const TextPanelFrb({super.key});

  @override
  State<TextPanelFrb> createState() => _TextPanelFrbState();
}

class _TextPanelFrbState extends State<TextPanelFrb>
    with TextTargetState<TextPanelFrb> {
  /// The installed families, once the engine has listed them.
  List<String>? _families;

  /// The faces of each family asked about so far.
  final Map<String, List<String>> _faces = {};

  /// Whether the less used controls are showing.
  bool _more = false;

  /// Whether the font list shows the variable fonts alone, and those
  /// families once the engine has listed them.
  bool _variableOnly = false;
  List<String>? _variableFamilies;

  @override
  void initState() {
    super.initState();
    textFontFamilies().then((families) {
      if (mounted) setState(() => _families = families);
    }).catchError((Object _) {
      // No list is a picker holding the built-in font alone.
      if (mounted) setState(() => _families = const []);
    });
  }

  /// The faces of [family], asked of the engine the first time and empty
  /// until the answer is in.
  List<String> _facesOf(String family) {
    if (family.isEmpty) return const [];
    final held = _faces[family];
    if (held != null) return held;
    _faces[family] = const [];
    textFontFaces(family: family).then((faces) {
      if (mounted) setState(() => _faces[family] = faces);
    }).catchError((Object _) {});
    return const [];
  }

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ListenableBuilder(
      listenable: Listenable.merge(
          [ui.model, ui.selectedLayers, ui.tools, fontAxesArrived]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final target = targetOf(ui);
        final document = target.document;
        final style = document.style;
        return styleMenu(ui, target, Container(
          color: t.surface0,
          child: ListView(
            children: [
              textTargetHeading(t, target.name),
              textGrid([
                textCell(t, l10n.textFont, _familyPicker(ui, target)),
                textCell(t, l10n.textFace, _facePicker(ui, target)),
                textCell(
                  t,
                  l10n.size,
                  _number(
                    ui,
                    target,
                    keyName: 'text-size',
                    value: document.size,
                    min: 1,
                    max: 2000,
                    decimals: 1,
                    suffix: ' px',
                    resetTo: plainTextSize,
                    change: (v) => (d) => d.copyWith(size: v),
                  ),
                ),
                textCell(
                  t,
                  l10n.textLeading,
                  _number(
                    ui,
                    target,
                    keyName: 'text-leading',
                    // Auto leading shows the number it comes to, and Reset
                    // goes back to it: that number is auto.
                    value: style.leading ?? document.size * 1.2,
                    min: 0,
                    max: 8000,
                    decimals: 1,
                    suffix: ' px',
                    resetTo: document.size * 1.2,
                    change: (v) => restyle((s) =>
                        (v - document.size * 1.2).abs() < 1e-5
                            ? s.copyWith(autoLeading: true)
                            : s.copyWith(leading: v)),
                  ),
                ),
                textCell(
                  t,
                  l10n.textKerning,
                  BareDropdown<BridgeKerning>(
                    key: const ValueKey('text-kerning'),
                    value: style.kerning,
                    options: BridgeKerning.values,
                    label: (k) => switch (k) {
                      BridgeKerning.off => l10n.off,
                      BridgeKerning.metrics => l10n.textKerningMetrics,
                    },
                    onChanged: (k) => commitText(
                        ui, target, restyle((s) => s.copyWith(kerning: k))),
                  ),
                ),
                textCell(
                  t,
                  l10n.textTracking,
                  _number(
                    ui,
                    target,
                    keyName: 'text-tracking',
                    value: style.tracking,
                    min: -1000,
                    max: 10000,
                    resetTo: plainTextStyle.tracking,
                    change: (v) => restyle((s) => s.copyWith(tracking: v)),
                  ),
                ),
                textCell(
                  t,
                  l10n.toolFill,
                  Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      HouseCheckbox(
                        key: const ValueKey('text-fill-on'),
                        value: style.fillOn,
                        onChanged: (on) => commitText(
                            ui, target, restyle((s) => s.copyWith(fillOn: on))),
                      ),
                      const SizedBox(width: 8),
                      _swatch(
                        keyName: 'text-fill',
                        colour: document.fill,
                        // White, as a new Text layer is set.
                        resetTo: const PickedColour(1, 1, 1),
                        onPicked: (c) =>
                            commitText(ui, target, (d) => d.copyWith(fill: c)),
                      ),
                    ],
                  ),
                ),
                textCell(
                  t,
                  l10n.toolStroke,
                  Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      HouseCheckbox(
                        key: const ValueKey('text-stroke-on'),
                        value: style.strokeOn,
                        onChanged: (on) => commitText(ui, target,
                            restyle((s) => s.copyWith(strokeOn: on))),
                      ),
                      const SizedBox(width: 8),
                      _swatch(
                        keyName: 'text-stroke',
                        colour: style.stroke,
                        resetTo: PickedColour(plainTextStyle.stroke.r,
                            plainTextStyle.stroke.g, plainTextStyle.stroke.b),
                        onPicked: (c) => commitText(
                            ui, target, restyle((s) => s.copyWith(stroke: c))),
                      ),
                    ],
                  ),
                ),
                // Only with a stroke to be that wide: a blank cell keeps the
                // rows under it where they were.
                if (style.strokeOn) ...[
                  const SizedBox.shrink(),
                  textCell(
                    t,
                    l10n.textStrokeWidth,
                    _number(
                      ui,
                      target,
                      keyName: 'text-stroke-width',
                      value: style.strokeWidth,
                      min: 0,
                      max: 500,
                      decimals: 1,
                      suffix: ' px',
                      resetTo: plainTextStyle.strokeWidth,
                      change: (v) => restyle((s) => s.copyWith(strokeWidth: v)),
                    ),
                  ),
                ],
              ]),
              _variableOnlyRow(t),
              ..._axisRows(t, ui, target),
              _moreTwirl(t),
              if (_more) ..._moreRows(t, ui, target),
            ],
          ),
        ));
      },
    );
  }

  /// The filter that keeps the font list to the families with axes.
  Widget _variableOnlyRow(LumitTheme t) => SizedBox(
        height: t.density.propertyRow,
        child: Row(
          children: [
            const SizedBox(width: 10),
            HouseCheckbox(
              key: const ValueKey('text-variable-only'),
              value: _variableOnly,
              onChanged: (on) {
                setState(() => _variableOnly = on);
                if (on && _variableFamilies == null) {
                  textVariableFontFamilies().then((families) {
                    if (mounted) setState(() => _variableFamilies = families);
                  }).catchError((Object _) {
                    if (mounted) setState(() => _variableFamilies = const []);
                  });
                }
              },
            ),
            const SizedBox(width: 8),
            Expanded(
              child: Text(l10n.textVariableOnly,
                  style: t.body.copyWith(color: t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis),
            ),
          ],
        ),
      );

  /// **A variable font's axes** for the whole layer, one number each, starting
  /// where the face sets them, and the Variable font spacing the animators'
  /// axes lay the letters out by. Nothing for a font that doesn't vary.
  List<Widget> _axisRows(LumitTheme t, LumitUiState ui, TextTarget target) {
    final style = target.document.style;
    final axes = shownFontAxes(style.family, style.face);
    if (axes.isEmpty) return const [];
    double valueOf(BridgeFontAxis axis) {
      for (final a in style.axes) {
        if (a.tag == axis.tag) return a.value;
      }
      return axis.face;
    }

    // An axis put back where the face sets it is left out of the file.
    TextChange set(BridgeFontAxis axis, double v) => restyle((s) => s.copyWith(
          axes: [
            for (final a in s.axes)
              if (a.tag != axis.tag) a,
            if ((v - axis.face).abs() > 1e-6)
              BridgeFontAxisValue(tag: axis.tag, value: v),
          ],
        ));
    return [
      SizedBox(
        height: t.density.propertyRow,
        child: Row(
          children: [
            const SizedBox(width: 10),
            Expanded(
              child: Text(l10n.textFontAxes,
                  style: t.body, maxLines: 1, overflow: TextOverflow.ellipsis),
            ),
            if (style.axes.isNotEmpty)
              HouseButton(
                key: const ValueKey('text-axes-reset'),
                small: true,
                frameless: true,
                onPressed: () => commitText(
                    ui, target, restyle((s) => s.copyWith(axes: const []))),
                child: Text(l10n.textFontAxesReset),
              ),
            const SizedBox(width: 10),
          ],
        ),
      ),
      textGrid([
        for (final axis in axes)
          textCell(
            t,
            axis.name,
            _number(
              ui,
              target,
              keyName: 'text-axis-${axis.tag}',
              value: valueOf(axis),
              min: axis.min,
              max: axis.max,
              decimals: axis.max - axis.min <= 10 ? 2 : 0,
              resetTo: axis.face,
              change: (v) => set(axis, v),
            ),
          ),
        textCell(
          t,
          l10n.textAxisSpacing,
          BareDropdown<BridgeAxisSpacing>(
            key: const ValueKey('text-axis-spacing'),
            value: style.axisSpacing,
            options: BridgeAxisSpacing.values,
            label: (s) => switch (s) {
              BridgeAxisSpacing.adaptive => l10n.textAxisSpacingAdaptive,
              BridgeAxisSpacing.perCharacter =>
                l10n.textAxisSpacingPerCharacter,
              BridgeAxisSpacing.default_ => l10n.textAxisSpacingDefault,
            },
            onChanged: (s) => commitText(
                ui, target, restyle((st) => st.copyWith(axisSpacing: s))),
          ),
        ),
      ]),
    ];
  }

  /// The way to the less used controls, the same twirl every fold in the app
  /// has, and on the same text edge as the labels above it.
  Widget _moreTwirl(LumitTheme t) => HoverMark(
        builder: (context, lit) => GestureDetector(
          key: const ValueKey('text-more'),
          behavior: HitTestBehavior.opaque,
          onTap: () => setState(() => _more = !_more),
          child: SizedBox(
            height: t.density.propertyRow,
            child: Row(
              children: [
                const SizedBox(width: 6),
                TwirlTurn(
                  open: _more,
                  child: lumitIcon(
                    _more ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                    size: iconSize,
                    color: Color.lerp(t.textMuted, t.textPrimary, lit)!,
                  ),
                ),
                const SizedBox(width: 2),
                Text(l10n.textMore,
                    style: t.body.copyWith(
                        color: Color.lerp(t.textMuted, t.textPrimary, lit))),
              ],
            ),
          ),
        ),
      );

  List<Widget> _moreRows(LumitTheme t, LumitUiState ui, TextTarget target) {
    final style = target.document.style;
    Widget toggle({
      required String keyName,
      required LumitIcon icon,
      required String tip,
      required bool on,
      required BridgeTextStyle Function(BridgeTextStyle) flip,
    }) =>
        LumitTooltip(
          message: tip,
          child: HouseButton(
            key: ValueKey<String>(keyName),
            small: true,
            frameless: !on,
            active: on,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: () => commitText(ui, target, restyle(flip)),
            child: lumitIcon(icon,
                size: iconSize, color: on ? t.textPrimary : t.textMuted),
          ),
        );
    Widget tick({
      required String keyName,
      required String label,
      required bool on,
      required BridgeTextStyle Function(BridgeTextStyle, bool) set,
    }) =>
        SizedBox(
          height: t.density.propertyRow,
          child: Row(
            children: [
              const SizedBox(width: 10),
              HouseCheckbox(
                key: ValueKey<String>(keyName),
                value: on,
                onChanged: (v) =>
                    commitText(ui, target, restyle((s) => set(s, v))),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(label,
                    style: t.body, maxLines: 1, overflow: TextOverflow.ellipsis),
              ),
            ],
          ),
        );
    return [
      textGrid([
        textCell(
          t,
          l10n.textScaleY,
          _number(
            ui,
            target,
            keyName: 'text-scale-y',
            value: style.scaleY,
            min: 1,
            max: 1000,
            suffix: ' %',
            resetTo: plainTextStyle.scaleY,
            change: (v) => restyle((s) => s.copyWith(scaleY: v)),
          ),
        ),
        textCell(
          t,
          l10n.textScaleX,
          _number(
            ui,
            target,
            keyName: 'text-scale-x',
            value: style.scaleX,
            min: 1,
            max: 1000,
            suffix: ' %',
            resetTo: plainTextStyle.scaleX,
            change: (v) => restyle((s) => s.copyWith(scaleX: v)),
          ),
        ),
        textCell(
          t,
          l10n.textBaselineShift,
          _number(
            ui,
            target,
            keyName: 'text-baseline-shift',
            value: style.baselineShift,
            min: -2000,
            max: 2000,
            decimals: 1,
            suffix: ' px',
            resetTo: plainTextStyle.baselineShift,
            change: (v) => restyle((s) => s.copyWith(baselineShift: v)),
          ),
        ),
      ]),
      // Four orders: stroke over, then all together. Only with both to order.
      if (style.fillOn && style.strokeOn)
       textCell(
        t,
        l10n.textFillAndStroke,
        BareDropdown<(bool, bool)>(
          key: const ValueKey('text-stroke-order'),
          value: (style.strokeOver, style.strokesTogether),
          options: const [
            (false, false),
            (true, false),
            (false, true),
            (true, true),
          ],
          label: (o) => switch (o) {
            (false, false) => l10n.textFillOverStroke,
            (true, false) => l10n.textStrokeOver,
            (false, true) => l10n.textAllFillsOverAllStrokes,
            (true, true) => l10n.textAllStrokesOverAllFills,
          },
          onChanged: (o) => commitText(
              ui,
              target,
              restyle(
                  (s) => s.copyWith(strokeOver: o.$1, strokesTogether: o.$2))),
        ),
      ),
      Padding(
        padding: const EdgeInsets.fromLTRB(10, 4, 10, 4),
        child: Wrap(
          spacing: 4,
          children: [
            toggle(
              keyName: 'text-faux-bold',
              icon: LumitIcon.textFauxBold,
              tip: l10n.tipTextFauxBold,
              on: style.fauxBold,
              flip: (s) => s.copyWith(fauxBold: !s.fauxBold),
            ),
            toggle(
              keyName: 'text-faux-italic',
              icon: LumitIcon.textFauxItalic,
              tip: l10n.tipTextFauxItalic,
              on: style.fauxItalic,
              flip: (s) => s.copyWith(fauxItalic: !s.fauxItalic),
            ),
            // The two capitals are one choice, and so are the two scripts:
            // pressing the lit one puts the letters back as typed.
            toggle(
              keyName: 'text-all-caps',
              icon: LumitIcon.textAllCaps,
              tip: l10n.tipTextAllCaps,
              on: style.caps == BridgeCaps.all,
              flip: (s) => s.copyWith(
                  caps: s.caps == BridgeCaps.all
                      ? BridgeCaps.normal
                      : BridgeCaps.all),
            ),
            toggle(
              keyName: 'text-small-caps',
              icon: LumitIcon.textSmallCaps,
              tip: l10n.tipTextSmallCaps,
              on: style.caps == BridgeCaps.small,
              flip: (s) => s.copyWith(
                  caps: s.caps == BridgeCaps.small
                      ? BridgeCaps.normal
                      : BridgeCaps.small),
            ),
            toggle(
              keyName: 'text-superscript',
              icon: LumitIcon.textSuperscript,
              tip: l10n.tipTextSuperscript,
              on: style.script == BridgeScript.superscript,
              flip: (s) => s.copyWith(
                  script: s.script == BridgeScript.superscript
                      ? BridgeScript.normal
                      : BridgeScript.superscript),
            ),
            toggle(
              keyName: 'text-subscript',
              icon: LumitIcon.textSubscript,
              tip: l10n.tipTextSubscript,
              on: style.script == BridgeScript.subscript,
              flip: (s) => s.copyWith(
                  script: s.script == BridgeScript.subscript
                      ? BridgeScript.normal
                      : BridgeScript.subscript),
            ),
          ],
        ),
      ),
      tick(
        keyName: 'text-ligatures',
        label: l10n.textLigatures,
        on: style.ligatures,
        set: (s, v) => s.copyWith(ligatures: v),
      ),
      tick(
        keyName: 'text-leading-auto',
        label: l10n.textLeadingAuto,
        on: style.leading == null,
        // Unticking keeps the number auto came to, so nothing moves until it
        // is changed.
        set: (s, v) => v
            ? s.copyWith(autoLeading: true)
            : s.copyWith(leading: target.document.size * 1.2),
      ),
    ];
  }

  /// A number that previews as it is dragged and writes once when let go.
  Widget _number(
    LumitUiState ui,
    TextTarget target, {
    required String keyName,
    required double value,
    required double min,
    required double max,
    required TextChange Function(double) change,
    int decimals = 0,
    String? suffix,
    double? resetTo,
  }) =>
      SizedBox(
        width: 72,
        child: DragValueField(
          key: ValueKey<String>(keyName),
          value: value,
          min: min,
          max: max,
          decimals: decimals,
          suffix: suffix,
          resetTo: resetTo,
          onChangeLive: (v) => previewText(ui, target, change(v.toDouble())),
          onChangeEnd: (v) => commitText(ui, target, change(v.toDouble())),
          onDragCancel: () => cancelTextPreview(ui, target),
          onChanged: (v) => commitText(ui, target, change(v.toDouble())),
        ),
      );

  /// A scene-linear colour as a chip that opens the picker. A text colour is
  /// chosen as a display colour, the same as a solid's.
  Widget _swatch({
    required String keyName,
    required BridgeColourRgba colour,
    required ValueChanged<BridgeColourRgba> onPicked,
    PickedColour? resetTo,
  }) {
    int byte(double f) => (f.clamp(0.0, 1.0) * 255).round();
    return ColourSwatchButton(
      key: ValueKey<String>(keyName),
      resetTo: resetTo,
      linear: true,
      colour:
          documentColour(byte(colour.r), byte(colour.g), byte(colour.b), 255),
      onPicked: (picked) => onPicked(BridgeColourRgba(
        r: picked.r,
        g: picked.g,
        b: picked.b,
        a: colour.a,
      )),
    );
  }

  /// The family picker: the built-in font first, then every installed family.
  /// A family the layer names and this machine lacks is listed too, so the
  /// picker says what the layer is asking for.
  Widget _familyPicker(LumitUiState ui, TextTarget target) {
    final family = target.document.style.family;
    final all = _families ?? const <String>[];
    final variable = _variableFamilies;
    // The layer's own family stays listed under the filter, so the picker
    // still says what the layer is set in.
    final installed = _variableOnly && variable != null
        ? [
            for (final f in all)
              if (variable.contains(f) || f == family) f
          ]
        : all;
    final missing = family.isNotEmpty && !installed.contains(family);
    final names = ['', ...installed, if (missing) family];
    final labels = [
      l10n.textFontBuiltIn,
      ...installed,
      if (missing) l10n.textFontMissing(family),
    ];
    return BareSearchDropdown(
      key: const ValueKey('text-family'),
      value: names.indexOf(family),
      options: labels,
      hint: l10n.textFontSearch,
      // Each family is listed in its own face. The built-in one is the Inter
      // the app carries, and a missing one has no face to show.
      familyOf: (i) =>
          i == 0 ? 'Inter' : (i <= installed.length ? names[i] : null),
      // A new family starts on its regular face, since the old family's face
      // may not exist in it.
      onChanged: (i) => commitText(
        ui,
        target,
        restyle((s) => s.copyWith(family: names[i], face: '')),
      ),
    );
  }

  /// The face picker. An unnamed face reads as the family's regular, which is
  /// what the engine draws for it.
  Widget _facePicker(LumitUiState ui, TextTarget target) {
    final style = target.document.style;
    final faces = _facesOf(style.family);
    final regular = faces.firstWhere(
      (f) => f.toLowerCase() == 'regular',
      orElse: () => faces.isEmpty ? l10n.textFaceRegular : faces.first,
    );
    final shown = style.face.isEmpty ? regular : style.face;
    return BareDropdown<String>(
      key: const ValueKey('text-face'),
      value: shown,
      options: faces.contains(shown) ? faces : [shown, ...faces],
      label: (f) => f,
      onChanged: faces.isEmpty
          ? null
          : (f) =>
              commitText(ui, target, restyle((s) => s.copyWith(face: f))),
    );
  }
}
