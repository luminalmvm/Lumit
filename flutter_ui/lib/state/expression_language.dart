// Which language an expression is written in, as the app shows and stores it.
//
// The engine keeps the choice beside each expression and never works it out
// from the text, so every place an expression is written has to say which
// language it is in. This file is what those places share: the name on screen,
// the word a settings file keeps, and the picker.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeTransformProp;

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// The name a language goes by on screen.
String expressionLanguageLabel(BridgeExpressionLanguage language) =>
    switch (language) {
      BridgeExpressionLanguage.rhai => l10n.expressionLanguageRhai,
      BridgeExpressionLanguage.javaScript => l10n.expressionLanguageJavaScript,
    };

/// The word a settings file keeps a language as.
String expressionLanguageId(BridgeExpressionLanguage language) =>
    switch (language) {
      BridgeExpressionLanguage.rhai => 'rhai',
      BridgeExpressionLanguage.javaScript => 'javascript',
    };

/// The language a stored word names. Anything else is Rhai, as nothing at all
/// is, which is what every expression was before there was a choice.
BridgeExpressionLanguage expressionLanguageOfId(Object? id) =>
    id == 'javascript'
        ? BridgeExpressionLanguage.javaScript
        : BridgeExpressionLanguage.rhai;

/// The grammar a language's text is coloured with, by the name the
/// highlighter knows it under.
String expressionGrammar(BridgeExpressionLanguage language) =>
    switch (language) {
      BridgeExpressionLanguage.rhai => 'rhai',
      BridgeExpressionLanguage.javaScript => 'javascript',
    };

/// A property an expression can name: a layer, one of its transform
/// properties, or an effect's row. [sameLayer] is whether the expression
/// is written on that layer, which is the short form.
class PropertyRef {
  final String layerName;
  final bool sameLayer;
  final BridgeTransformProp? transform;

  /// The effect's heading name and the row's label as the effect names it,
  /// for an effect's row.
  final String? effectName;
  final String? rowLabel;

  const PropertyRef.layer(this.layerName, {this.sameLayer = false})
      : transform = null,
        whole = false,
        effectName = null,
        rowLabel = null;

  /// A row of two or three numbers named as a whole, where the language has
  /// a word for the pair: JavaScript's `position`, not `position[0]`.
  final bool whole;

  const PropertyRef.transform(this.layerName, this.transform,
      {this.sameLayer = false, this.whole = false})
      : effectName = null,
        rowLabel = null;

  const PropertyRef.row(this.layerName, this.effectName, this.rowLabel,
      {this.sameLayer = false})
      : transform = null,
        whole = false;
}

/// The reference as [language] writes it, or null when the language has no
/// way to say it: Rhai reaches a layer and its transform, not an effect's
/// rows.
String? referenceText(PropertyRef ref, BridgeExpressionLanguage language) {
  String quoted(String s) => '"${s.replaceAll(r'\', r'\\').replaceAll('"', r'\"')}"';
  switch (language) {
    case BridgeExpressionLanguage.javaScript:
      final layer =
          ref.sameLayer ? 'thisLayer' : 'thisComp.layer(${quoted(ref.layerName)})';
      if (ref.transform case final prop?) {
        var name = _jsTransform(prop);
        if (name == null) return null;
        // The pair as one: the index goes.
        if (ref.whole) name = name.replaceAll(RegExp(r'\[\d\]$'), '');
        return '$layer.$name';
      }
      if (ref.effectName case final effect?) {
        final row = 'effect(${quoted(effect)})(${quoted(ref.rowLabel!)})';
        return ref.sameLayer ? row : '$layer.$row';
      }
      return layer;
    case BridgeExpressionLanguage.rhai:
      final layer = ref.sameLayer ? 'layer()' : 'layer(${quoted(ref.layerName)})';
      if (ref.transform case final prop?) {
        final name = _rhaiTransform(prop);
        return name == null ? null : '$layer.$name';
      }
      if (ref.effectName != null) return null;
      return layer;
  }
}

String? _jsTransform(BridgeTransformProp prop) => switch (prop) {
      BridgeTransformProp.positionX => 'position[0]',
      BridgeTransformProp.positionY => 'position[1]',
      BridgeTransformProp.positionZ => 'position[2]',
      BridgeTransformProp.anchorX => 'anchorPoint[0]',
      BridgeTransformProp.anchorY => 'anchorPoint[1]',
      BridgeTransformProp.anchorZ => 'anchorPoint[2]',
      BridgeTransformProp.scaleX => 'scale[0]',
      BridgeTransformProp.scaleY => 'scale[1]',
      BridgeTransformProp.scaleZ => 'scale[2]',
      BridgeTransformProp.rotation => 'rotation',
      BridgeTransformProp.rotationX => 'xRotation',
      BridgeTransformProp.rotationY => 'yRotation',
      BridgeTransformProp.opacity => 'opacity',
      BridgeTransformProp.orientationX => 'orientation[0]',
      BridgeTransformProp.orientationY => 'orientation[1]',
      BridgeTransformProp.orientationZ => 'orientation[2]',
      BridgeTransformProp.poiX => 'pointOfInterest[0]',
      BridgeTransformProp.poiY => 'pointOfInterest[1]',
      BridgeTransformProp.poiZ => 'pointOfInterest[2]',
      BridgeTransformProp.zoom => 'zoom',
      _ => null,
    };

String? _rhaiTransform(BridgeTransformProp prop) => switch (prop) {
      BridgeTransformProp.positionX => 'x',
      BridgeTransformProp.positionY => 'y',
      BridgeTransformProp.positionZ => 'z',
      BridgeTransformProp.anchorX => 'anchor_x',
      BridgeTransformProp.anchorY => 'anchor_y',
      BridgeTransformProp.anchorZ => 'anchor_z',
      BridgeTransformProp.scaleX => 'scale_x',
      BridgeTransformProp.scaleY => 'scale_y',
      BridgeTransformProp.scaleZ => 'scale_z',
      BridgeTransformProp.rotation => 'rotation',
      BridgeTransformProp.rotationX => 'rotation_x',
      BridgeTransformProp.rotationY => 'rotation_y',
      BridgeTransformProp.opacity => 'opacity',
      BridgeTransformProp.zoom => 'zoom',
      _ => null,
    };

/// The languages in the order every list of them is drawn.
const List<BridgeExpressionLanguage> expressionLanguages = [
  BridgeExpressionLanguage.javaScript,
  BridgeExpressionLanguage.rhai,
];

/// The dropdown that picks an expression's language. [dense] is the in-row
/// face, for a bar or a panel's foot.
class ExpressionLanguagePicker extends StatelessWidget {
  final BridgeExpressionLanguage value;
  final ValueChanged<BridgeExpressionLanguage> onChanged;
  final bool dense;

  const ExpressionLanguagePicker({
    super.key,
    required this.value,
    required this.onChanged,
    this.dense = false,
  });

  @override
  Widget build(BuildContext context) => BareDropdown<BridgeExpressionLanguage>(
        value: value,
        options: expressionLanguages,
        label: expressionLanguageLabel,
        dense: dense,
        onChanged: onChanged,
      );
}
