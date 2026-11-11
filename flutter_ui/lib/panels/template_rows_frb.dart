// The rows of the Graphics template panel: what it lists, and the value each
// control shows, edited in place with the controls Effect controls uses.

import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/dropper.dart';
import '../theme/theme.dart';
import '../widgets/angle_dial.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart'
    show DropperButton, effectCellWidth, withUnitRider;
import 'fx_section.dart' show fxRowHeight;

/// One line of the panel's list.
sealed class TemplateLine {
  const TemplateLine();
}

/// A group's folding heading.
class TemplateGroupLine extends TemplateLine {
  final String name;
  final bool open;
  const TemplateGroupLine(this.name, {required this.open});
}

/// A control.
class TemplateControlLine extends TemplateLine {
  final BridgeControl control;
  const TemplateControlLine(this.control);
}

/// What a copy may change about a text control beside its words.
enum TemplateTextRow { font, size, colour }

/// One of a text control's further rows, under it.
class TemplateTextRowLine extends TemplateLine {
  final BridgeControl control;
  final TemplateTextRow which;
  const TemplateTextRowLine(this.control, this.which);
}

/// One option of a menu control, under it.
class TemplateOptionLine extends TemplateLine {
  final BridgeControl control;
  final int option;
  const TemplateOptionLine(this.control, this.option);
}

/// The line under a menu's options that adds one.
class TemplateAddOptionLine extends TemplateLine {
  final BridgeControl control;
  const TemplateAddOptionLine(this.control);
}

/// The groups of [controls], in the order each first appears.
List<String> templateGroups(List<BridgeControl> controls) {
  final groups = <String>[];
  for (final control in controls) {
    final group = control.group;
    if (group != null && !groups.contains(group)) groups.add(group);
  }
  return groups;
}

/// The panel's list: the controls with no group, in the template's order,
/// then each group under its heading. A menu in [openMenus] shows its
/// options, and one in [ownMenus] the line that adds another.
List<TemplateLine> templateLines(
  List<BridgeControl> controls, {
  required Set<String> shutGroups,
  required Set<String> openMenus,
  required Set<String> ownMenus,
}) {
  final lines = <TemplateLine>[];
  void add(BridgeControl control) {
    lines.add(TemplateControlLine(control));
    if (control.gone) return;
    if (control.kind == BridgeControlKind.text) {
      if (control.font) {
        lines.add(TemplateTextRowLine(control, TemplateTextRow.font));
      }
      if (control.size) {
        lines.add(TemplateTextRowLine(control, TemplateTextRow.size));
      }
      if (control.colour) {
        lines.add(TemplateTextRowLine(control, TemplateTextRow.colour));
      }
    }
    if (control.kind == BridgeControlKind.menu &&
        openMenus.contains(control.id)) {
      for (var i = 0; i < control.options.length; i++) {
        lines.add(TemplateOptionLine(control, i));
      }
      if (ownMenus.contains(control.id)) {
        lines.add(TemplateAddOptionLine(control));
      }
    }
  }

  for (final control in controls) {
    if (control.group == null) add(control);
  }
  for (final group in templateGroups(controls)) {
    final open = !shutGroups.contains(group);
    lines.add(TemplateGroupLine(group, open: open));
    if (!open) continue;
    for (final control in controls) {
      if (control.group == group) add(control);
    }
  }
  return lines;
}

/// A name being typed in place of a label: the whole of it selected, kept on
/// Enter or a click away, and thrown away on Escape.
class TemplateRenameField extends StatefulWidget {
  final String initial;
  final ValueChanged<String> onDone;
  final VoidCallback onCancel;
  const TemplateRenameField({
    super.key,
    required this.initial,
    required this.onDone,
    required this.onCancel,
  });

  @override
  State<TemplateRenameField> createState() => _TemplateRenameFieldState();
}

class _TemplateRenameFieldState extends State<TemplateRenameField> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.initial,
  )..selection = TextSelection(
      baseOffset: 0,
      extentOffset: widget.initial.length,
    );

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => HouseTextField(
        key: const ValueKey('template-rename-field'),
        controller: _controller,
        width: double.infinity,
        autofocus: true,
        submitOnLostFocus: true,
        onSubmitted: widget.onDone,
        onCancelled: widget.onCancel,
      );
}

/// A swatch of a colour kept scene-linear, as a colour row's is drawn.
Widget templateSwatch(LumitTheme t, List<double> colour, {double lit = 0}) {
  int byte(int i) =>
      ((i < colour.length ? colour[i] : 0.0).clamp(0.0, 1.0) * 255).round();
  return Container(
    width: 28,
    height: 18,
    decoration: BoxDecoration(
      color: documentColour(byte(0), byte(1), byte(2), 255),
      borderRadius: BorderRadius.circular(t.tokens.wellRadius),
      border: Border.all(
          color: Color.lerp(t.hairlineStrong, t.textPrimary, 0.7 * lit)!),
    ),
  );
}

/// A value shown and not edited: plain mono, in [ink].
Widget templateReadout(LumitTheme t, String text, {Color? ink}) => Text(
      text,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      textDirection: TextDirection.ltr,
      style: t.mono.copyWith(fontSize: wellTextSize, color: ink ?? t.textMuted),
    );

/// A well that names a picture and takes nothing: the glyph and [name].
Widget templateWell(LumitTheme t, String name) => Container(
      height: wellHeight,
      padding: const EdgeInsets.symmetric(horizontal: 5),
      decoration: BoxDecoration(
        color: t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.wellRadius),
        border: Border.all(color: t.hairline),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          glyph.LumitIcon(LumitIcons.footage, size: 12, colour: t.textMuted),
          const SizedBox(width: 5),
          Flexible(
            child: Text(name,
                maxLines: 1, overflow: TextOverflow.ellipsis, style: t.small),
          ),
        ],
      ),
    );

/// What a control's row holds in the template, edited in place.
///
/// A number, an angle and a point send each step of a drag: the value well
/// groups them into one undo step. A slider, a dial and a colour send once,
/// on release.
class TemplateValue extends StatelessWidget {
  final BridgeControl control;

  /// The menu's options are the person's own words, not an effect's.
  final bool ownMenu;
  final ValueChanged<BridgeControlValue> onSet;

  const TemplateValue({
    super.key,
    required this.control,
    required this.ownMenu,
    required this.onSet,
  });

  String _option(int i) => i >= control.options.length
      ? ''
      : (ownMenu ? control.options[i] : engineLabel(control.options[i]));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (control.gone) {
      return glyph.LumitIcon(LumitIcons.missing,
          key: ValueKey<String>('template-gone-${control.id}'),
          size: 14,
          colour: t.warning);
    }
    final value = control.value;
    if (value == null) return Text('—', style: t.small);
    if (control.animated) return _held(t, value);
    return switch (value) {
      BridgeControlValue_Number(:final field0) =>
        control.kind == BridgeControlKind.angle
            ? _AngleValue(id: control.id, degrees: field0, onSet: onSet)
            : _number(context, t, field0),
      BridgeControlValue_Point(:final field0) => _point(field0),
      BridgeControlValue_Colour(:final field0) => _ColourValue(
          id: control.id,
          label: control.label,
          colour: field0,
          onSet: onSet,
        ),
      BridgeControlValue_Tick(:final field0) => SizedBox(
          width: effectCellWidth,
          child: Align(
            alignment: AlignmentDirectional.centerStart,
            child: HouseCheckbox(
              key: ValueKey<String>('template-tick-${control.id}'),
              value: field0,
              onChanged: (on) => onSet(BridgeControlValue.tick(on)),
            ),
          ),
        ),
      BridgeControlValue_Choice(:final field0) => control.options.isEmpty
          ? Text('—', style: t.small)
          : SizedBox(
              width: effectCellWidth + 40,
              child: BareDropdown<int>(
                key: ValueKey<String>('template-choice-${control.id}'),
                value: field0 < control.options.length ? field0 : 0,
                options: [for (var i = 0; i < control.options.length; i++) i],
                label: _option,
                onChanged: (i) => onSet(BridgeControlValue.choice(i)),
              ),
            ),
      BridgeControlValue_Text(:final field0) =>
        _TextValue(id: control.id, text: field0, onSet: onSet),
      BridgeControlValue_Font(:final family, :final face) => _FontValue(
          id: control.id,
          family: family,
          face: face,
          onSet: onSet,
        ),
      BridgeControlValue_Picture() => templateWell(t, control.layerName),
    };
  }

  /// The value of a row that is keyed or has an expression: read, in the ink
  /// a keyed value wears, and not set.
  Widget _held(LumitTheme t, BridgeControlValue value) => switch (value) {
        BridgeControlValue_Number(:final field0) => withUnitRider(
            t,
            control.unit,
            templateReadout(
                t,
                control.kind == BridgeControlKind.angle
                    ? '${field0.toStringAsFixed(1)}°'
                    : field0.toStringAsFixed(2),
                ink: t.animated)),
        BridgeControlValue_Point(:final field0) => templateReadout(
            t, [for (final v in field0) v.toStringAsFixed(1)].join(', '),
            ink: t.animated),
        BridgeControlValue_Colour(:final field0) => templateSwatch(t, field0),
        BridgeControlValue_Tick(:final field0) =>
          HouseCheckbox(value: field0, onChanged: null),
        BridgeControlValue_Choice(:final field0) =>
          Text(_option(field0),
              overflow: TextOverflow.ellipsis, style: t.small),
        BridgeControlValue_Text(:final field0) => Text(field0,
            maxLines: 1, overflow: TextOverflow.ellipsis, style: t.small),
        BridgeControlValue_Font(:final family, :final face) => Text(
            [if (family.isEmpty) l10n.textFontBuiltIn else family, face]
                .where((s) => s.isNotEmpty)
                .join(' '),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: t.small),
        BridgeControlValue_Picture() => templateWell(t, control.layerName),
      };

  Widget _number(BuildContext context, LumitTheme t, double value) {
    // A range the wrong way round bounds nothing.
    final bounded = control.ranged && control.rangeMax >= control.rangeMin;
    final low = bounded ? control.rangeMin : -1000000.0;
    final high = bounded ? control.rangeMax : 1000000.0;
    final from = math.max(control.sliderMin, low);
    final to = math.min(control.sliderMax, high);
    final span = (to - from).abs();
    void set(num next) => onSet(
        BridgeControlValue.number(next.toDouble().clamp(low, high).toDouble()));
    final field = SizedBox(
      width: effectCellWidth,
      child: DragValueField(
        key: ValueKey<String>('template-number-${control.id}'),
        value: value.clamp(low, high),
        min: low,
        max: high,
        // Paced by the slider's travel, as an effect's number is.
        speed: span <= 0 ? 0.5 : span / 200,
        decimals: 2,
        onChanged: set,
        onChangeLive: set,
        onChangeEnd: set,
      ),
    );
    final sliders = Provider.of<LumitUiState>(context, listen: false)
        .workspace
        .interface
        .rangeSliders;
    if (!sliders || !(to > from)) return withUnitRider(t, control.unit, field);
    final both = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        field,
        const SizedBox(width: 6),
        HouseSlider(
          key: ValueKey<String>('template-slider-${control.id}'),
          value: value.clamp(from, to).toDouble(),
          min: from,
          max: to,
          showValue: false,
          width: 78,
          commitOnRelease: true,
          onChanged: set,
        ),
      ],
    );
    // Without room for both, the number stands alone.
    return withUnitRider(
      t,
      control.unit,
      LayoutBuilder(
          builder: (context, box) =>
              box.maxWidth < effectCellWidth + 6 + 78 ? field : both),
    );
  }

  Widget _point(Float64List point) {
    final axes = control.axes == 3 ? 3 : 2;
    double at(int axis) => axis < point.length ? point[axis] : 0.0;
    void set(int axis, num next) => onSet(BridgeControlValue.point(
          Float64List.fromList([
            for (var a = 0; a < axes; a++) a == axis ? next.toDouble() : at(a),
          ]),
        ));
    Widget field(int axis) => Flexible(
          child: SizedBox(
            width: effectCellWidth,
            child: DragValueField(
              key: ValueKey<String>('template-point-${control.id}-$axis'),
              value: at(axis),
              min: -1000000,
              max: 1000000,
              speed: 1,
              decimals: 1,
              onChanged: (v) => set(axis, v),
              onChangeLive: (v) => set(axis, v),
              onChangeEnd: (v) => set(axis, v),
            ),
          ),
        );
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var axis = 0; axis < axes; axis++) ...[
          if (axis > 0) const SizedBox(width: 4),
          field(axis),
        ],
      ],
    );
  }
}

/// An angle: turns and degrees, and the dial beside them when there is room.
class _AngleValue extends StatefulWidget {
  final String id;
  final double degrees;
  final ValueChanged<BridgeControlValue> onSet;
  const _AngleValue(
      {required this.id, required this.degrees, required this.onSet});

  @override
  State<_AngleValue> createState() => _AngleValueState();
}

class _AngleValueState extends State<_AngleValue> {
  /// Where the dial is while it is being turned. It is sent on release.
  double? _turning;

  @override
  void didUpdateWidget(_AngleValue old) {
    super.didUpdateWidget(old);
    if (old.degrees != widget.degrees) _turning = null;
  }

  void _set(double degrees) => widget.onSet(BridgeControlValue.number(degrees));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final fields = TurnsAndDegreesField(
      keyName: 'template-${widget.id}',
      degrees: widget.degrees,
      onChanged: _set,
      onCommit: _set,
    );
    final dial = fxRowHeight(t);
    return LayoutBuilder(
      builder: (context, box) => box.maxWidth < TurnsAndDegreesField.width
          ? SizedBox(
              width: effectCellWidth,
              child: DragValueField(
                key: ValueKey<String>('template-angle-${widget.id}'),
                value: widget.degrees,
                min: -1000000,
                max: 1000000,
                decimals: 1,
                suffix: '°',
                onChanged: (v) => _set(v.toDouble()),
                onChangeLive: (v) => _set(v.toDouble()),
                onChangeEnd: (v) => _set(v.toDouble()),
              ),
            )
          : box.maxWidth < TurnsAndDegreesField.width + 6 + dial
              ? fields
              : Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    fields,
                    const SizedBox(width: 6),
                    AngleDial(
                      key: ValueKey<String>('template-dial-${widget.id}'),
                      size: dial,
                      degrees: _turning ?? widget.degrees,
                      onChanged: (v) => setState(() => _turning = v),
                      onChangeEnd: _set,
                    ),
                  ],
                ),
    );
  }
}

/// A colour: the swatch that opens the picker, and the dropper.
class _ColourValue extends StatelessWidget {
  final String id;
  final String label;
  final Float64List colour;
  final ValueChanged<BridgeControlValue> onSet;
  const _ColourValue({
    required this.id,
    required this.label,
    required this.colour,
    required this.onSet,
  });

  double _at(int i) => i < colour.length ? colour[i] : (i == 3 ? 1.0 : 0.0);

  /// The picked colour with the row's own alpha, which no swatch sets.
  BridgeControlValue _with(double r, double g, double b) =>
      BridgeControlValue.colour(Float64List.fromList([r, g, b, _at(3)]));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        GestureDetector(
          key: ValueKey<String>('template-colour-$id'),
          behavior: HitTestBehavior.opaque,
          onTap: () async {
            final box = context.findRenderObject();
            if (box is! RenderBox) return;
            await showColourPicker(
              context: context,
              position: box.localToGlobal(Offset(0, box.size.height + 4)),
              initial: PickedColour(_at(0), _at(1), _at(2)),
              // Scene-linear, as the row keeps it, and as far past white as
              // the colour already goes.
              scale: ColourScale.unit,
              max: [1.0, _at(0), _at(1), _at(2)].reduce(math.max),
              onCommit: (picked) => onSet(_with(picked.r, picked.g, picked.b)),
            );
          },
          child: HoverMark(
            swells: true,
            builder: (_, lit) => templateSwatch(t, colour, lit: lit),
          ),
        ),
        const SizedBox(width: 6),
        DropperButton(
          id: 'template-$id',
          tip: l10n.tipSampleFromViewer,
          arm: (ui) => ui.armDropper(DropperArm(
            id: 'template-$id',
            reads: DropperReads.colour,
            label: label,
            onPick: (sample) => onSet(_with(sample.r, sample.g, sample.b)),
          )),
        ),
      ],
    );
  }
}

/// A text control's words: a box as tall as its lines. Enter keeps them, as
/// a click away does. Shift and Enter starts a line. Escape puts back what
/// was there.
class _TextValue extends StatefulWidget {
  final String id;
  final String text;
  final ValueChanged<BridgeControlValue> onSet;
  const _TextValue(
      {required this.id, required this.text, required this.onSet});

  @override
  State<_TextValue> createState() => _TextValueState();
}

class _TextValueState extends State<_TextValue> {
  late final TextEditingController _controller =
      TextEditingController(text: widget.text);
  final FocusNode _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _controller.addListener(_grow);
  }

  @override
  void didUpdateWidget(_TextValue old) {
    super.didUpdateWidget(old);
    if (old.text != widget.text && !_focus.hasFocus) {
      _controller.text = widget.text;
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _grow() {
    if (mounted) setState(() {});
  }

  void _keep() {
    if (_controller.text != widget.text) {
      widget.onSet(BridgeControlValue.text(_controller.text));
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    const padding = EdgeInsets.symmetric(horizontal: 6, vertical: 3);
    return LayoutBuilder(builder: (context, box) {
      // As tall as the words are at this width, and never less than a line.
      final painter = TextPainter(
        text: TextSpan(
            text: _controller.text.isEmpty ? ' ' : _controller.text,
            style: t.small),
        textDirection: Directionality.of(context),
        textScaler: MediaQuery.textScalerOf(context),
      )..layout(
          // The well's own edge takes a pixel either side.
          maxWidth: math.max(
              0,
              (box.maxWidth.isFinite ? box.maxWidth : 200) -
                  padding.horizontal -
                  2));
      final height =
          math.max(wellHeight, painter.height + padding.vertical + 2);
      painter.dispose();
      return Focus(
        canRequestFocus: false,
        skipTraversal: true,
        onKeyEvent: (_, event) {
          if (event is! KeyDownEvent) return KeyEventResult.ignored;
          final enter = event.logicalKey == LogicalKeyboardKey.enter ||
              event.logicalKey == LogicalKeyboardKey.numpadEnter;
          if (enter && !HardwareKeyboard.instance.isShiftPressed) {
            _keep();
            _focus.unfocus();
            return KeyEventResult.handled;
          }
          return KeyEventResult.ignored;
        },
        child: SizedBox(
          height: height,
          child: HouseTextField(
            key: ValueKey<String>('template-text-${widget.id}'),
            controller: _controller,
            focusNode: _focus,
            multiline: true,
            topAlign: true,
            style: t.small,
            padding: padding,
            width: double.infinity,
            onTapOutside: _keep,
            onCancelled: () {
              _controller.text = widget.text;
              _focus.unfocus();
            },
          ),
        ),
      );
    });
  }
}

/// A font: the family and the face, as the Text panel picks them.
class _FontValue extends StatefulWidget {
  final String id;
  final String family;
  final String face;
  final ValueChanged<BridgeControlValue> onSet;
  const _FontValue({
    required this.id,
    required this.family,
    required this.face,
    required this.onSet,
  });

  @override
  State<_FontValue> createState() => _FontValueState();
}

class _FontValueState extends State<_FontValue> {
  /// The installed families, once the engine has listed them.
  List<String>? _families;

  /// The faces of each family asked about so far.
  final Map<String, List<String>> _faces = {};

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
    final family = widget.family;
    final installed = _families ?? const <String>[];
    final missing = family.isNotEmpty && !installed.contains(family);
    final names = ['', ...installed, if (missing) family];
    final labels = [
      l10n.textFontBuiltIn,
      ...installed,
      if (missing) l10n.textFontMissing(family),
    ];
    final faces = _facesOf(family);
    final regular = faces.firstWhere(
      (f) => f.toLowerCase() == 'regular',
      orElse: () => faces.isEmpty ? l10n.textFaceRegular : faces.first,
    );
    final shown = widget.face.isEmpty ? regular : widget.face;
    // Side by side with room, and one under the other without.
    return Wrap(
      spacing: 4,
      runSpacing: 2,
      children: [
        SizedBox(
          width: effectCellWidth + 40,
          child: BareSearchDropdown(
            key: ValueKey<String>('template-family-${widget.id}'),
            value: names.indexOf(family),
            options: labels,
            hint: l10n.textFontSearch,
            familyOf: (i) =>
                i == 0 ? 'Inter' : (i <= installed.length ? names[i] : null),
            // A new family starts on its regular face.
            onChanged: (i) => widget
                .onSet(BridgeControlValue.font(family: names[i], face: '')),
          ),
        ),
        SizedBox(
          width: effectCellWidth,
          child: BareDropdown<String>(
            key: ValueKey<String>('template-face-${widget.id}'),
            value: shown,
            options: faces.contains(shown) ? faces : [shown, ...faces],
            label: (f) => f,
            onChanged: faces.isEmpty
                ? null
                : (f) => widget
                    .onSet(BridgeControlValue.font(family: family, face: f)),
          ),
        ),
      ],
    );
  }
}
