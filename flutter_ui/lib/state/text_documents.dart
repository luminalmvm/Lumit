// Copies of a text document with one thing changed.
//
// A text layer's document is written whole, so anything a writer leaves out is
// deleted. Every writer starts from the document the layer has and changes
// the one field it is editing.

import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:uuid/uuid.dart';

/// The style of a layer nobody has styled, for a document built by hand. The
/// engine's `defaultTextStyle` is the authority, and a test holds the two
/// together.
const BridgeTextStyle plainTextStyle = BridgeTextStyle(
  family: '',
  face: '',
  kerning: BridgeKerning.off,
  tracking: 0,
  scaleX: 100,
  scaleY: 100,
  baselineShift: 0,
  caps: BridgeCaps.normal,
  script: BridgeScript.normal,
  fauxBold: false,
  fauxItalic: false,
  ligatures: true,
  fillOn: true,
  strokeOn: false,
  stroke: BridgeColourRgba(r: 0, g: 0, b: 0, a: 1),
  strokeWidth: 1,
  strokeOver: true,
);

/// The paragraph of a layer nobody has styled, held the same way.
const BridgeParagraphStyle plainParagraphStyle = BridgeParagraphStyle(
  align: BridgeTextAlign.left,
  justify: false,
  indentLeft: 0,
  indentRight: 0,
  indentFirst: 0,
  spaceBefore: 0,
  spaceAfter: 0,
);

extension TextDocumentCopy on BridgeTextDocument {
  /// [clearPath] lays the line straight again, since a null [path] means
  /// "leave it alone".
  BridgeTextDocument copyWith({
    String? text,
    String? expression,
    double? size,
    BridgeColourRgba? fill,
    UuidValue? path,
    bool clearPath = false,
    BridgeScalar? pathOffset,
    List<BridgeTextAnimator>? animators,
    BridgeTextStyle? style,
    BridgeParagraphStyle? paragraph,
  }) =>
      BridgeTextDocument(
        text: text ?? this.text,
        expression: expression ?? this.expression,
        size: size ?? this.size,
        fill: fill ?? this.fill,
        path: clearPath ? null : (path ?? this.path),
        pathOffset: pathOffset ?? this.pathOffset,
        animators: animators ?? this.animators,
        style: style ?? this.style,
        paragraph: paragraph ?? this.paragraph,
      );
}

extension TextStyleCopy on BridgeTextStyle {
  /// [autoLeading] puts the leading back to auto, since a null [leading]
  /// means "leave it alone".
  BridgeTextStyle copyWith({
    String? family,
    String? face,
    double? leading,
    bool autoLeading = false,
    BridgeKerning? kerning,
    double? tracking,
    double? scaleX,
    double? scaleY,
    double? baselineShift,
    BridgeCaps? caps,
    BridgeScript? script,
    bool? fauxBold,
    bool? fauxItalic,
    bool? ligatures,
    bool? fillOn,
    bool? strokeOn,
    BridgeColourRgba? stroke,
    double? strokeWidth,
    bool? strokeOver,
  }) =>
      BridgeTextStyle(
        family: family ?? this.family,
        face: face ?? this.face,
        leading: autoLeading ? null : (leading ?? this.leading),
        kerning: kerning ?? this.kerning,
        tracking: tracking ?? this.tracking,
        scaleX: scaleX ?? this.scaleX,
        scaleY: scaleY ?? this.scaleY,
        baselineShift: baselineShift ?? this.baselineShift,
        caps: caps ?? this.caps,
        script: script ?? this.script,
        fauxBold: fauxBold ?? this.fauxBold,
        fauxItalic: fauxItalic ?? this.fauxItalic,
        ligatures: ligatures ?? this.ligatures,
        fillOn: fillOn ?? this.fillOn,
        strokeOn: strokeOn ?? this.strokeOn,
        stroke: stroke ?? this.stroke,
        strokeWidth: strokeWidth ?? this.strokeWidth,
        strokeOver: strokeOver ?? this.strokeOver,
      );
}

extension ParagraphStyleCopy on BridgeParagraphStyle {
  /// [noBox] goes back to point text, since a null [boxWidth] means "leave it
  /// alone".
  BridgeParagraphStyle copyWith({
    BridgeTextAlign? align,
    double? boxWidth,
    bool noBox = false,
    bool? justify,
    double? indentLeft,
    double? indentRight,
    double? indentFirst,
    double? spaceBefore,
    double? spaceAfter,
  }) =>
      BridgeParagraphStyle(
        align: align ?? this.align,
        boxWidth: noBox ? null : (boxWidth ?? this.boxWidth),
        justify: justify ?? this.justify,
        indentLeft: indentLeft ?? this.indentLeft,
        indentRight: indentRight ?? this.indentRight,
        indentFirst: indentFirst ?? this.indentFirst,
        spaceBefore: spaceBefore ?? this.spaceBefore,
        spaceAfter: spaceAfter ?? this.spaceAfter,
      );
}
