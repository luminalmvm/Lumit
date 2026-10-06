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
      listenable: Listenable.merge([ui.model, ui.selectedLayers, ui.tools]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final target = targetOf(ui);
        final document = target.document;
        final style = document.style;
        return Container(
          color: t.surface0,
          child: ListView(
            children: [
              textTargetHeading(t, target.name ?? l10n.textNewText),
              textCell(t, l10n.textFont, _familyPicker(ui, target)),
              textCell(t, l10n.textFace, _facePicker(ui, target)),
              textGrid([
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
                    // Auto leading shows the number it comes to.
                    value: style.leading ?? document.size * 1.2,
                    min: 0,
                    max: 8000,
                    decimals: 1,
                    suffix: ' px',
                    change: (v) => restyle((s) => s.copyWith(leading: v)),
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
                        onPicked: (c) => commitText(
                            ui, target, restyle((s) => s.copyWith(stroke: c))),
                      ),
                    ],
                  ),
                ),
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
                    change: (v) => restyle((s) => s.copyWith(strokeWidth: v)),
                  ),
                ),
              ]),
              if (_more) ..._moreRows(t, ui, target),
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 6, 10, 10),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: HouseButton(
                    key: const ValueKey('text-more'),
                    small: true,
                    onPressed: () => setState(() => _more = !_more),
                    child: Text(_more ? l10n.textLess : l10n.textMore),
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }

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
            change: (v) => restyle((s) => s.copyWith(baselineShift: v)),
          ),
        ),
      ]),
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
        keyName: 'text-stroke-over',
        label: l10n.textStrokeOver,
        on: style.strokeOver,
        set: (s, v) => s.copyWith(strokeOver: v),
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
  }) {
    int byte(double f) => (f.clamp(0.0, 1.0) * 255).round();
    return ColourSwatchButton(
      key: ValueKey<String>(keyName),
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
    final installed = _families ?? const <String>[];
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
