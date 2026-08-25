// The rows for what a layer is *made of*, above its Transform.
//
// A text layer gets its words, size and fill; a camera gets its zoom; a solid
// gets the asset's colour and size. Which rows appear is decided by asking the
// layer, so a footage layer simply has none.
//
// **The solid row says who else it affects, and means it.** A solid is an asset
// in the Project panel, not a per-layer setting, so recolouring one recolours
// every layer drawing it. That is the useful behaviour — one edit repaints every
// backdrop — but it is a surprise if the row does not say so, which is why it
// does.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';
import 'package:lumit_flutter/src/rust/api/solid.dart';
import 'package:lumit_flutter/widgets/autofill.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'fx_section.dart';

/// The section of source rows for [layer], or nothing when its kind has none.
class SourceRowsFrb extends StatefulWidget {
  final LayerReference layer;
  final VoidCallback onChanged;

  /// Whether the section is twirled open, and how to toggle it — held by the
  /// panel so the open set survives a rebuild of these rows.
  final bool open;
  final VoidCallback onToggle;

  const SourceRowsFrb({
    super.key,
    required this.layer,
    required this.onChanged,
    required this.open,
    required this.onToggle,
  });

  @override
  State<SourceRowsFrb> createState() => _SourceRowsFrbState();
}

class _SourceRowsFrbState extends State<SourceRowsFrb> {
  TextEditingController? _text;
  TextEditingController? _expression;

  @override
  void dispose() {
    _text?.dispose();
    _expression?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final text = widget.layer.getText();
    final zoom = widget.layer.getCameraZoom();
    final solid = _solidOf(widget.layer);

    final rows = <Widget>[
      if (text != null) ..._textRows(t, text),
      if (zoom != null) _zoomRow(t, zoom),
      if (solid != null) ..._solidRows(t, solid),
      ..._retimeRows(t),
    ];
    if (rows.isEmpty) return const SizedBox.shrink();

    return FxSection(
      title: l10n.sourceSection,
      open: widget.open,
      onToggle: widget.onToggle,
      rows: rows,
    );
  }

  /// The solid asset behind a solid layer, if that is what this layer is.
  SolidReference? _solidOf(LayerReference layer) {
    final item = layer.getSourceItem();
    return item is ItemReference_Solid ? item.field0 : null;
  }

  List<Widget> _textRows(LumitTheme t, BridgeTextDocument document) {
    // The controller is created against the document the layer currently has,
    // and rebuilt only when the text changed underneath us — otherwise typing
    // would fight the rebuild its own commit triggers.
    if (_text == null ||
        (_text!.text != document.text && !_text!.selection.isValid)) {
      _text?.dispose();
      _text = TextEditingController(text: document.text);
    }
    final expression = document.expression ?? '';
    if (_expression == null ||
        (_expression!.text != expression && !_expression!.selection.isValid)) {
      _expression?.dispose();
      _expression = ExpressionTextEditingController(text: expression);
    }

    void write({
      String? body,
      String? expression,
      double? size,
      BridgeColourRgba? fill,
    }) {
      widget.layer.setText(
        document: BridgeTextDocument(
          text: body ?? _text!.text,
          // An empty box is no expression at all, which the engine settles —
          // so emptying the field simply hands the layer back to its words.
          expression: expression ?? _expression!.text,
          size: size ?? document.size,
          fill: fill ?? document.fill,
        ),
      );
      widget.onChanged();
    }

    return [
      _row(
        t,
        l10n.sourceText,
        SizedBox(
          width: _cellWidth + 60,
          child: HouseTextField(
            key: const ValueKey('src-text'),
            controller: _text!,
            width: _cellWidth + 60,
            onSubmitted: (value) => write(body: value),
          ),
        ),
      ),
      // The words can come from an expression instead — the same language the
      // numeric properties use, printed rather than measured, which is how a
      // caption shows a live value. The Text box above stays as it was: it is
      // what the layer says again once this one is empty.
      _row(
        t,
        'Expression',
        SizedBox(
          //width: _cellWidth + 60,
          child: HouseTextField(
            key: const ValueKey('src-text-expression'),
            controller: _expression!,
            width: double.infinity,
            style: t.mono,
            submitOnLostFocus: true,
            autofill: ExpressionAutofillGenerator(),
            onSubmitted: (value) => write(expression: value),
          ),
        ),
      ),
      _row(
        t,
        l10n.size,
        SizedBox(
          width: _cellWidth,
          child: DragValueField(
            key: const ValueKey('src-text-size'),
            value: document.size,
            min: 1,
            max: 2000,
            decimals: 1,
            onChanged: (v) => write(size: v.toDouble()),
          ),
        ),
      ),
      _row(
        t,
        l10n.toolFill,
        _swatch(
          t,
          keyName: 'src-text-fill',
          colour: document.fill,
          onPicked: (c) => write(fill: c),
        ),
      ),
    ];
  }

  Widget _zoomRow(LumitTheme t, BridgeScalar zoom) {
    if (zoom is! BridgeScalar_Static) {
      return _row(
        t,
        l10n.sourceZoom,
        Text(l10n.animated, style: t.small.copyWith(color: t.textMuted)),
      );
    }
    return _row(
      t,
      l10n.sourceZoom,
      SizedBox(
        width: _cellWidth,
        child: DragValueField(
          key: const ValueKey('src-camera-zoom'),
          value: zoom.field0,
          min: 1,
          max: 100000,
          speed: 4,
          decimals: 0,
          onChanged: (v) {
            widget.layer
                .setCameraZoom(zoom: BridgeScalar.static_(v.toDouble()));
            widget.onChanged();
          },
        ),
      ),
    );
  }

  List<Widget> _solidRows(LumitTheme t, SolidReference solid) {
    final definition = solid.getDefinition();

    void write({BridgeColourRgba? colour, int? width, int? height}) {
      solid.setDefinition(
        definition: BridgeSolidDef(
          name: definition.name,
          colour: colour ?? definition.colour,
          width: width ?? definition.width,
          height: height ?? definition.height,
        ),
      );
      widget.onChanged();
    }

    return [
      _row(
        t,
        l10n.sourceSolidColour,
        _swatch(
          t,
          keyName: 'src-solid-colour',
          colour: definition.colour,
          onPicked: (c) => write(colour: c),
        ),
      ),
      _row(
        t,
        l10n.sourceSolidSize,
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              width: _cellWidth,
              child: DragValueField(
                key: const ValueKey('src-solid-width'),
                value: definition.width,
                min: 1,
                max: 16384,
                onChanged: (v) => write(width: v.toInt()),
              ),
            ),
            const SizedBox(width: 6),
            SizedBox(
              width: _cellWidth,
              child: DragValueField(
                key: const ValueKey('src-solid-height'),
                value: definition.height,
                min: 1,
                max: 16384,
                onChanged: (v) => write(height: v.toInt()),
              ),
            ),
          ],
        ),
      ),
      Padding(
        padding: const EdgeInsets.only(top: 2),
        child: Text(
          l10n.sourceAssetNote,
          style: t.small.copyWith(color: t.textMuted),
        ),
      ),
    ];
  }

  /// How in-between frames are made — the one retiming-adjacent control that
  /// belongs on a card rather than in a graph.
  ///
  /// This card used to carry a whole second retiming system beside it: an
  /// enable switch, a constant speed and a reverse gate, writing a segment
  /// store that rivalled the Retime property. K-249 deleted it. Retiming is
  /// **Ctrl+Alt+T** and the Retime graph now, which is the only place a ramp
  /// was ever editable anyway; what is left here was never part of the map
  /// (docs/04 §10) and applies whether or not the layer is retimed.
  ///
  /// Shown on footage only. Every layer *has* the setting — it is a plain
  /// field with a default, and the engine asks any layer for it — but a layer
  /// with no frames of its own has no in-betweens to make, and a row that
  /// changes nothing is worse than no row. It is also what puts the Source
  /// card on screen at all, so an offer here would give an adjustment layer a
  /// source card describing a source it does not have.
  ///
  /// **Flow is not one of the choices here (K-331).** It used to be a third
  /// entry in this dropdown, which made it look like a peer of Nearest and
  /// Blend — a small setting you pick and forget. It is not: it carries eight
  /// parameters of its own and is the most expensive thing a layer can ask for.
  /// It is the Flow switch in the layer's switch cluster instead, which reveals
  /// the Flow group (K-088). Choosing Nearest or Blend here turns it off, since
  /// they are the same setting underneath.
  List<Widget> _retimeRows(LumitTheme t) {
    if (widget.layer.getKind() != BridgeLayerKind.footage) return const [];
    // Flow has its own switch; a layer that is on it shows Nearest here, which
    // is what it falls back to whenever flow cannot help.
    const choices = [BridgeRetimeInterp.nearest, BridgeRetimeInterp.blend];
    final live = widget.layer.getInterpolation();
    return [
      _row(
        t,
        l10n.sourceInBetweenFrames,
        SizedBox(
          width: _cellWidth + 40,
          child: BareDropdown<BridgeRetimeInterp>(
            key: const ValueKey('src-retime-interp'),
            value: choices.contains(live) ? live : BridgeRetimeInterp.nearest,
            options: choices,
            label: (i) => switch (i) {
              BridgeRetimeInterp.nearest => l10n.interpNearest,
              BridgeRetimeInterp.blend => l10n.interpBlend,
              BridgeRetimeInterp.flow => l10n.interpOpticalFlow,
            },
            onChanged: (i) {
              widget.layer.setInterpolation(interpolation: i);
              widget.onChanged();
            },
          ),
        ),
      ),
    ];
  }

  Widget _swatch(
    LumitTheme t, {
    required String keyName,
    required BridgeColourRgba colour,
    required ValueChanged<BridgeColourRgba> onPicked,
  }) {
    int byte(double f) => (f.clamp(0.0, 1.0) * 255).round();
    final shown =
        documentColour(byte(colour.r), byte(colour.g), byte(colour.b), 255);

    return SizedBox(
      width: _cellWidth,
      child: Align(
        alignment: Alignment.centerLeft,
        child: GestureDetector(
          key: ValueKey<String>(keyName),
          behavior: HitTestBehavior.opaque,
          onTap: () async {
            final box = context.findRenderObject();
            if (box is! RenderBox) return;
            await showColourPicker(
              context: context,
              position: box.localToGlobal(Offset(0, box.size.height + 4)),
              initial: PickedColour.of(shown),
              // A solid's colour is chosen as a display colour, so its
              // channels read 0–255.
              scale: ColourScale.bytes,
              // It applies as it is chosen — there is no cheaper preview of a
              // solid than the solid itself.
              onCommit: (picked) => onPicked(BridgeColourRgba(
                r: picked.r,
                g: picked.g,
                b: picked.b,
                a: colour.a,
              )),
            );
          },
          child: MouseRegion(
            cursor: SystemMouseCursors.click,
            child: Container(
              width: 28,
              height: 18,
              decoration: BoxDecoration(
                color: shown,
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                border: Border.all(color: t.hairlineStrong),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _row(LumitTheme t, String label, Widget control) => fxTwoColumnRow(
        context: context,
        // A source row is not a keyable property, so its name is plain text —
        // there is no curve for the graph editor to aim at.
        name: Text(label, style: t.body, overflow: TextOverflow.ellipsis),
        control: control,
      );
}

/// Matches the Effect controls panel's own cell width, so the two sections'
/// values line up down the panel.
const double _cellWidth = 78;
