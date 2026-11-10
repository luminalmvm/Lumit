// Caption styles: the looks that come with Lumit, the ones a person saves,
// putting one on a caption layer, and the tile that shows each.
//
// A style is nothing but a text document: font, fill, outline, plate and the
// animators that follow the words. Putting one on a caption layer writes that
// document and keeps the layer's box width, its place and its captions, so a
// restyle never loses a word's timing.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/captions.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import '../l10n/strings.dart';
import '../state/text_documents.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'text_animation_presets.dart';
import 'text_animator_rows_frb.dart';

/// The looks that come with Lumit.
enum CaptionStyle {
  plain,
  box,
  highlight,
  karaoke,
  pop,
  wordBox,
  oneWord;

  String get title => switch (this) {
        plain => l10n.captionStylePlain,
        box => l10n.captionStyleBox,
        highlight => l10n.captionStyleHighlight,
        karaoke => l10n.captionStyleKaraoke,
        pop => l10n.captionStylePop,
        wordBox => l10n.captionStyleWordBox,
        oneWord => l10n.captionStyleOneWord,
      };
}

const BridgeColourRgba _white = BridgeColourRgba(r: 1, g: 1, b: 1, a: 1);
const BridgeColourRgba _black = BridgeColourRgba(r: 0, g: 0, b: 0, a: 1);

/// [style]'s look for a composition [height] pixels tall: a size that reads
/// at that height, the bundled Inter, and the animators it is made of. The
/// box is the engine's to fit to the composition.
BridgeTextDocument captionLook(CaptionStyle style, {required int height}) {
  final h = height <= 0 ? 1080.0 : height.toDouble();
  final size = (h *
          switch (style) {
            CaptionStyle.oneWord => 0.1,
            CaptionStyle.wordBox => 0.058,
            _ => 0.052,
          })
      .roundToDouble();
  final heavy = style == CaptionStyle.wordBox || style == CaptionStyle.oneWord;
  var text = plainTextStyle.copyWith(
    family: 'Inter',
    face: heavy ? 'Black' : 'Bold',
    kerning: BridgeKerning.metrics,
    caps: style == CaptionStyle.wordBox ? BridgeCaps.all : BridgeCaps.normal,
  );
  text = style == CaptionStyle.box
      ? text.copyWith(
          plateOn: true,
          plateLines: true,
          platePadding: (size * 0.22).roundToDouble(),
          plateRadius: (size * 0.12).roundToDouble(),
        )
      // The outline is drawn under the fill, so it only thickens the edge.
      : text.copyWith(
          strokeOn: true,
          stroke: _black,
          strokeWidth: math.max(1, (size * 0.08).roundToDouble()),
          strokeOver: false,
        );
  CaptionAnimationPreset? preset = switch (style) {
    CaptionStyle.highlight => CaptionAnimationPreset.highlightWord,
    CaptionStyle.karaoke => CaptionAnimationPreset.karaoke,
    CaptionStyle.pop ||
    CaptionStyle.oneWord =>
      CaptionAnimationPreset.popWordsIn,
    CaptionStyle.wordBox => CaptionAnimationPreset.boxWord,
    _ => null,
  };
  return BridgeTextDocument(
    text: '',
    size: size,
    fill: _white,
    pathOffset: const BridgeScalar.static_(0),
    animators: [
      if (preset != null) captionPresetAnimator(preset),
      _emphasis(style),
    ],
    style: text,
    paragraph: plainParagraphStyle.copyWith(
        align: BridgeTextAlign.centre, vertical: BridgeVerticalAlign.bottom),
  );
}

/// The animator every built-in look ends with: a word picked out by hand
/// takes a colour. Yellow, or orange where yellow already marks the word
/// being said.
BridgeTextAnimator _emphasis(CaptionStyle style) {
  final fresh = freshTextAnimator(l10n.textAnimatorFollowsEmphasis);
  final timed =
      style == CaptionStyle.highlight || style == CaptionStyle.karaoke;
  return animatorWith(fresh,
      selector: selectorWith(fresh.selector,
          mode: BridgeSelectorMode.emphasis, basis: BridgeSelectorBasis.words),
      fillR: const BridgeScalar.static_(0),
      fillG: BridgeScalar.static_(timed ? -0.5 : -0.2),
      fillB: const BridgeScalar.static_(-0.9));
}

/// A look a person kept with Save style.
class SavedCaptionStyle {
  final String name;
  final BridgeTextDocument look;

  /// Kept from a layer that showed one word at a time, so it does again.
  final bool single;
  const SavedCaptionStyle(this.name, this.look, {this.single = false});
}

/// One of the looks on offer: a built-in one or a saved one.
class CaptionLookChoice {
  final CaptionStyle? builtIn;
  final SavedCaptionStyle? saved;
  const CaptionLookChoice.builtIn(CaptionStyle this.builtIn) : saved = null;
  const CaptionLookChoice.saved(SavedCaptionStyle this.saved) : builtIn = null;

  /// What the choice is remembered by.
  String get key => builtIn?.name ?? 'saved:${saved!.name}';
  String get title => builtIn?.title ?? saved!.name;

  /// Shows one word at a time, and sets the limits to say so.
  bool get single =>
      builtIn == CaptionStyle.oneWord || (saved?.single ?? false);

  BridgeTextDocument look({required int height}) =>
      saved?.look ?? captionLook(builtIn!, height: height);
}

/// Every look on offer: the built-in ones, then the saved ones in the order
/// they were saved.
List<CaptionLookChoice> captionLookChoices(Workspace workspace) => [
      for (final style in CaptionStyle.values) CaptionLookChoice.builtIn(style),
      for (final saved in savedCaptionStyles(workspace))
        CaptionLookChoice.saved(saved),
    ];

/// The choice remembered as [key], or Plain when it is gone.
CaptionLookChoice captionLookChoice(Workspace workspace, String? key) {
  for (final choice in captionLookChoices(workspace)) {
    if (choice.key == key) return choice;
  }
  return const CaptionLookChoice.builtIn(CaptionStyle.plain);
}

// --- Putting a look on a layer ----------------------------------------------

/// Restyle caption [layer] as [choice], as one undo step. Its box width, its
/// place and its captions stay. A one-word look cuts the captions again a word
/// at a time; moving off one cuts them again at the language's usual limits.
void applyCaptionLook(ProjectReference? project, CompositionReference comp,
    LayerReference layer, CaptionLookChoice choice) {
  final now = layer.getText();
  final captions = layer.captions();
  if (now == null || captions == null) return;
  var limits = captions.limits;
  if (choice.single) {
    limits = limits.copyWith(words: 1, lines: 1);
  } else if (limits.words == 1) {
    final usual = comp.captionLimitsFor(language: captions.language);
    limits = limits.copyWith(words: usual.words, lines: usual.lines);
  }
  final look = choice.look(height: comp.getSettings().height);
  final leading = look.style.leading ?? look.size * 1.2;
  final document = look.copyWith(
    text: now.text,
    paragraph: look.paragraph.copyWith(
      boxWidth: now.paragraph.boxWidth,
      boxHeight: leading * limits.lines,
      vertical: BridgeVerticalAlign.bottom,
    ),
  );
  asOneUndoStep(project, () {
    layer.setText(document: document);
    if (limits != captions.limits) {
      layer.setCaptionLimits(limits: limits, rebuild: true);
    }
  }, name: 'Edit captions');
}

/// The key of the look [document] is wearing, when it is exactly one of
/// [choices], for the tile to outline. Size, font, colours and animators
/// count; the box does not, since every layer's is its own.
String? captionLookKeyOf(
    BridgeTextDocument document, List<CaptionLookChoice> choices,
    {required int height}) {
  final worn = _lookKey(document);
  for (final choice in choices) {
    if (_lookKey(choice.look(height: height)) == worn) return choice.key;
  }
  return null;
}

String _lookKey(BridgeTextDocument d) => _encodeLook(d).toString();

extension CaptionLimitsCopy on BridgeCaptionLimits {
  BridgeCaptionLimits copyWith({
    int? charsPerLine,
    int? lines,
    int? words,
    int? minDurationMs,
    int? maxDurationMs,
    int? gapFrames,
    int? readingSpeed,
    BridgeSpeakerLabels? speakerLabels,
  }) =>
      BridgeCaptionLimits(
        charsPerLine: charsPerLine ?? this.charsPerLine,
        lines: lines ?? this.lines,
        words: words ?? this.words,
        minDurationMs: minDurationMs ?? this.minDurationMs,
        maxDurationMs: maxDurationMs ?? this.maxDurationMs,
        gapFrames: gapFrames ?? this.gapFrames,
        readingSpeed: readingSpeed ?? this.readingSpeed,
        splitOnSpeaker: splitOnSpeaker,
        speakerLabels: speakerLabels ?? this.speakerLabels,
      );
}

// --- Saved styles -------------------------------------------------------------

/// The saved looks, as the settings file holds them.
List<SavedCaptionStyle> savedCaptionStyles(Workspace workspace) => [
      for (final raw in workspace.captionStyles)
        if (raw['name'] case final String name)
          if (raw['look'] case final Map<String, dynamic> look)
            SavedCaptionStyle(name, _decodeLook(look),
                single: raw['single'] == true),
    ];

/// Keep [layer]'s look under [name], replacing one already called that.
void saveCaptionStyle(Workspace workspace, String name, LayerReference layer) {
  final look = layer.getText();
  final captions = layer.captions();
  if (look == null) return;
  workspace.setCaptionStyles([
    for (final raw in workspace.captionStyles)
      if (raw['name'] != name) raw,
    {
      'name': name,
      'single': captions?.limits.words == 1,
      'look': _encodeLook(look),
    },
  ]);
}

/// Call the saved look [from] by [to]. One already called [to] goes.
void renameCaptionStyle(Workspace workspace, String from, String to) =>
    workspace.setCaptionStyles([
      for (final raw in workspace.captionStyles)
        if (raw['name'] == from)
          {...raw, 'name': to}
        else if (raw['name'] != to)
          raw,
    ]);

void deleteCaptionStyle(Workspace workspace, String name) =>
    workspace.setCaptionStyles([
      for (final raw in workspace.captionStyles)
        if (raw['name'] != name) raw,
    ]);

List<double> _rgba(BridgeColourRgba c) => [c.r, c.g, c.b, c.a];
BridgeColourRgba _colour(Object? raw, BridgeColourRgba fallback) =>
    raw is List && raw.length == 4 && raw.every((v) => v is num)
        ? BridgeColourRgba(
            r: (raw[0] as num).toDouble(),
            g: (raw[1] as num).toDouble(),
            b: (raw[2] as num).toDouble(),
            a: (raw[3] as num).toDouble())
        : fallback;

/// A number a saved look keeps. A keyed one keeps its first key: a look is a
/// still thing, and its moving parts are the words.
Object _scalar(BridgeScalar s) => switch (s) {
      BridgeScalar_Static(:final field0) => field0,
      BridgeScalar_Keyframed(:final field0) =>
        field0.isEmpty ? 0.0 : field0.first.value,
      BridgeScalar_Expression(:final field0, :final field1) => {
          'expression': field0,
          'language': field1.name,
        },
    };

BridgeScalar _scalarOf(Object? raw, double fallback) => switch (raw) {
      num() => BridgeScalar.static_(raw.toDouble()),
      {'expression': final String text, 'language': final String language} =>
        BridgeScalar.expression(
            text,
            BridgeExpressionLanguage.values.asNameMap()[language] ??
                BridgeExpressionLanguage.javaScript),
      _ => BridgeScalar.static_(fallback),
    };

Map<String, dynamic> _encodeLook(BridgeTextDocument d) {
  final s = d.style;
  return {
    'size': d.size,
    'fill': _rgba(d.fill),
    'align': d.paragraph.align.name,
    'style': {
      'family': s.family,
      'face': s.face,
      'leading': s.leading,
      'kerning': s.kerning.name,
      'tracking': s.tracking,
      'scale_x': s.scaleX,
      'scale_y': s.scaleY,
      'baseline_shift': s.baselineShift,
      'caps': s.caps.name,
      'faux_bold': s.fauxBold,
      'faux_italic': s.fauxItalic,
      'ligatures': s.ligatures,
      'fill_on': s.fillOn,
      'stroke_on': s.strokeOn,
      'stroke': _rgba(s.stroke),
      'stroke_width': s.strokeWidth,
      'stroke_over': s.strokeOver,
      'strokes_together': s.strokesTogether,
      'plate_on': s.plateOn,
      'plate': _rgba(s.plate),
      'plate_padding': s.platePadding,
      'plate_radius': s.plateRadius,
      'plate_lines': s.plateLines,
    },
    'animators': [
      for (final a in d.animators)
        {
          'name': a.name,
          'basis': a.selector.basis.name,
          'shape': a.selector.shape.name,
          'mode': a.selector.mode.name,
          'ease': a.selector.ease,
          for (final value in TextAnimatorValue.values)
            value.name: _scalar(textAnimatorScalarOf(a, value)),
        },
    ],
  };
}

BridgeTextDocument _decodeLook(Map<String, dynamic> j) {
  double number(Map<String, dynamic> m, String key, double fallback) =>
      (m[key] as num?)?.toDouble() ?? fallback;
  bool flag(Map<String, dynamic> m, String key, bool fallback) =>
      m[key] is bool ? m[key] as bool : fallback;
  T named<T extends Enum>(List<T> values, Object? raw, T fallback) =>
      values.asNameMap()[raw] ?? fallback;
  final s = j['style'] is Map<String, dynamic>
      ? j['style'] as Map<String, dynamic>
      : const <String, dynamic>{};
  const p = plainTextStyle;
  final style = BridgeTextStyle(
    family: s['family'] is String ? s['family'] as String : p.family,
    face: s['face'] is String ? s['face'] as String : p.face,
    leading: (s['leading'] as num?)?.toDouble(),
    kerning: named(BridgeKerning.values, s['kerning'], p.kerning),
    tracking: number(s, 'tracking', p.tracking),
    scaleX: number(s, 'scale_x', p.scaleX),
    scaleY: number(s, 'scale_y', p.scaleY),
    baselineShift: number(s, 'baseline_shift', p.baselineShift),
    caps: named(BridgeCaps.values, s['caps'], p.caps),
    script: p.script,
    fauxBold: flag(s, 'faux_bold', p.fauxBold),
    fauxItalic: flag(s, 'faux_italic', p.fauxItalic),
    ligatures: flag(s, 'ligatures', p.ligatures),
    fillOn: flag(s, 'fill_on', p.fillOn),
    strokeOn: flag(s, 'stroke_on', p.strokeOn),
    stroke: _colour(s['stroke'], p.stroke),
    strokeWidth: number(s, 'stroke_width', p.strokeWidth),
    strokeOver: flag(s, 'stroke_over', p.strokeOver),
    strokesTogether: flag(s, 'strokes_together', p.strokesTogether),
    plateOn: flag(s, 'plate_on', p.plateOn),
    plate: _colour(s['plate'], p.plate),
    platePadding: number(s, 'plate_padding', p.platePadding),
    plateRadius: number(s, 'plate_radius', p.plateRadius),
    plateLines: flag(s, 'plate_lines', p.plateLines),
  );
  final animators = <BridgeTextAnimator>[
    if (j['animators'] case final List<dynamic> list)
      for (final raw in list.whereType<Map<String, dynamic>>())
        () {
          var a = freshTextAnimator(raw['name'] as String? ?? '');
          a = animatorWith(a,
              selector: selectorWith(a.selector,
                  basis: named(BridgeSelectorBasis.values, raw['basis'],
                      a.selector.basis),
                  shape: named(BridgeSelectorShape.values, raw['shape'],
                      a.selector.shape),
                  mode: named(
                      BridgeSelectorMode.values, raw['mode'], a.selector.mode),
                  ease: number(raw, 'ease', a.selector.ease)));
          for (final value in TextAnimatorValue.values) {
            a = animatorWithScalar(a, value,
                _scalarOf(raw[value.name], textAnimatorDefault(value)));
          }
          return a;
        }(),
  ];
  return BridgeTextDocument(
    text: '',
    size: number(j, 'size', plainTextSize),
    fill: _colour(j['fill'], _white),
    pathOffset: const BridgeScalar.static_(0),
    animators: animators,
    style: style,
    paragraph: plainParagraphStyle.copyWith(
      align: named(BridgeTextAlign.values, j['align'], BridgeTextAlign.centre),
      vertical: BridgeVerticalAlign.bottom,
    ),
  );
}

// --- The tile -------------------------------------------------------------------

/// A look's sample: a short line in that look over the picture's surround,
/// with the middle word as the one being said. Drawn here from the look's own
/// numbers, so a saved style shows as truly as a built-in one.
class CaptionLookTile extends StatelessWidget {
  final CaptionLookChoice choice;
  final bool selected;
  final VoidCallback onPressed;
  final void Function(Offset at)? onMenu;
  final double width;
  const CaptionLookTile({
    super.key,
    required this.choice,
    required this.selected,
    required this.onPressed,
    this.onMenu,
    this.width = 96,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final radius = BorderRadius.circular(t.tokens.actionRadius);
    return SizedBox(
      width: width,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onPressed,
        onSecondaryTapDown:
            onMenu == null ? null : (d) => onMenu!(d.globalPosition),
        child: HoverMark(
          cursor: SystemMouseCursors.click,
          builder: (context, lit) => Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              AnimatedContainer(
                duration: ThemeScope.of(context).motion.rowTrail.duration,
                height: width * 9 / 16,
                decoration: BoxDecoration(
                  color: t.viewerSurround,
                  borderRadius: radius,
                ),
                foregroundDecoration: BoxDecoration(
                  borderRadius: radius,
                  border: Border.all(
                    color: selected
                        ? t.accent
                        : Color.lerp(t.hairline, t.hairlineStrong, lit)!,
                    width: selected ? 2 : 1,
                  ),
                ),
                padding: const EdgeInsets.symmetric(horizontal: 6),
                alignment: Alignment.center,
                child: FittedBox(
                  fit: BoxFit.scaleDown,
                  child: _Sample(
                    look: choice.look(height: 1080),
                    single: choice.single,
                  ),
                ),
              ),
              const SizedBox(height: 3),
              Text(
                choice.title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: t.small.copyWith(
                    color: selected
                        ? t.textPrimary
                        : Color.lerp(t.textMuted, t.textPrimary, lit)),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Which words of the sample an animator reaches: the first has been said,
/// the second is being said, the third is still to come.
bool _reaches(BridgeTextAnimator a, int word) => switch (a.selector.mode) {
      BridgeSelectorMode.range => true,
      BridgeSelectorMode.current => word == 1,
      BridgeSelectorMode.said => word <= 1,
      BridgeSelectorMode.unsaid => word == 2,
      BridgeSelectorMode.emphasis => false,
    };

double _static(BridgeScalar s, double fallback) => switch (s) {
      BridgeScalar_Static(:final field0) => field0,
      BridgeScalar_Keyframed(:final field0) =>
        field0.isEmpty ? fallback : field0.first.value,
      _ => fallback,
    };

Color _colourOf(BridgeColourRgba c, [double alpha = 1]) => Color.from(
    alpha: (c.a * alpha).clamp(0, 1),
    red: c.r.clamp(0, 1),
    green: c.g.clamp(0, 1),
    blue: c.b.clamp(0, 1));

class _Sample extends StatelessWidget {
  final BridgeTextDocument look;
  final bool single;
  const _Sample({required this.look, required this.single});

  @override
  Widget build(BuildContext context) {
    final style = look.style;
    final words = l10n.captionStyleSample.split(' ');
    // The tile is a picture of the look, not of its size: one height for
    // every sample, and a little larger for a word on its own.
    final size = single ? 30.0 : 18.0;
    final scale = size / (look.size <= 0 ? 1 : look.size);
    final weight = switch (style.face.toLowerCase()) {
      'black' => FontWeight.w900,
      'bold' => FontWeight.w700,
      'medium' => FontWeight.w500,
      _ => FontWeight.w400,
    };
    final text = TextStyle(
      fontFamily: 'Inter',
      fontSize: size,
      fontWeight: weight,
      height: 1.2,
      letterSpacing: style.tracking / 1000 * size,
      decoration: TextDecoration.none,
    );
    Widget word(int index) {
      var fill = look.fill;
      var opacity = 1.0;
      var grow = 1.0;
      BridgeColourRgba? box;
      var boxAlpha = 0.0;
      for (final a in look.animators) {
        if (!_reaches(a, index)) continue;
        fill = BridgeColourRgba(
          r: fill.r + _static(a.fillR, 0),
          g: fill.g + _static(a.fillG, 0),
          b: fill.b + _static(a.fillB, 0),
          a: fill.a,
        );
        opacity *= _static(a.opacity, 100) / 100;
        grow *= _static(a.scaleY, 100) / 100;
        final boxOpacity = _static(a.boxOpacity, 0) / 100;
        if (boxOpacity > 0) {
          box = BridgeColourRgba(
              r: _static(a.boxR, 1),
              g: _static(a.boxG, 0.75),
              b: _static(a.boxB, 0.15),
              a: 1);
          boxAlpha = boxOpacity;
        }
      }
      var said = words[index];
      if (style.caps == BridgeCaps.all) said = said.toUpperCase();
      final strokeWidth = style.strokeWidth * scale;
      final face = Text(said,
          style: text.copyWith(
              color: style.fillOn ? _colourOf(fill) : const Color(0x00000000)));
      final outline = style.strokeOn && strokeWidth > 0
          ? Text(said,
              style: text.copyWith(
                  foreground: Paint()
                    ..style = PaintingStyle.stroke
                    ..strokeJoin = StrokeJoin.round
                    ..strokeWidth = strokeWidth * 2
                    ..color = _colourOf(style.stroke)))
          : null;
      Widget drawn = Stack(children: [
        if (outline != null && !style.strokeOver) outline,
        face,
        if (outline != null && style.strokeOver) outline,
      ]);
      if (box != null) {
        drawn = DecoratedBox(
          decoration: BoxDecoration(
            color: _colourOf(box, boxAlpha),
            borderRadius: BorderRadius.circular(size * 0.15),
          ),
          child: Padding(
            padding: EdgeInsets.symmetric(horizontal: size * 0.12),
            child: drawn,
          ),
        );
      }
      return Opacity(
        opacity: opacity.clamp(0, 1),
        child: Transform.scale(scale: grow.clamp(0, 2), child: drawn),
      );
    }

    Widget line = Row(
      mainAxisSize: MainAxisSize.min,
      children: single
          ? [word(1)]
          : [
              for (var i = 0; i < words.length && i < 3; i++) ...[
                if (i > 0) SizedBox(width: size * 0.28),
                word(i),
              ],
            ],
    );
    if (style.plateOn) {
      line = DecoratedBox(
        decoration: BoxDecoration(
          color: _colourOf(style.plate),
          borderRadius: BorderRadius.circular(style.plateRadius * scale),
        ),
        child: Padding(
          padding: EdgeInsets.symmetric(
              horizontal: style.platePadding * scale,
              vertical: style.platePadding * scale * 0.5),
          child: line,
        ),
      );
    }
    return line;
  }
}
