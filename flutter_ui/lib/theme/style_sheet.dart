// A style of your own: a small stylesheet that dresses Lumit's parts.
//
// A style file is written like CSS. `@base` picks Studio, Desk or Lantern to
// start from, `:root` sets the style's variables (corners, gaps, faces), and
// each rule dresses one kind of part, everywhere or only inside one panel:
//
//   @base lantern;
//   :root { --control-radius: 4; }
//   pane { background: var(--surface1) 60%; radius: 14; }
//   timeline button:hover { background: #ffffff22; }
//
// It dresses and never lays out: where things sit stays Lumit's. Reading
// never throws, a line it can't use is skipped and counted, so a typo costs a
// line rather than the whole style.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/strings.dart';

import 'theme.dart';
import 'theme_tokens.dart';

/// The extension a style file is written under.
const String styleFileExtension = 'lumstyle';

/// One part of a selector: `timeline`, `button.primary:hover`.
class _Compound {
  final String? name;
  final Set<String> classes;
  final Set<String> states;
  const _Compound(this.name, this.classes, this.states);

  int get weight =>
      (name == null ? 0 : 1) + 10 * (classes.length + states.length);
}

class _Rule {
  final List<_Compound> selector;
  final Map<String, String> props;
  final int weight;
  final int order;
  _Rule(this.selector, this.props, this.order)
      : weight = selector.fold(0, (sum, c) => sum + c.weight);
}

/// How a style dresses one part. Every field is null where the style says
/// nothing, so the part keeps its own look.
class PartStyle {
  final Color? background;
  final Color? borderColour;
  final double? borderWidth;
  final double? radius;
  final List<BoxShadow>? shadows;
  final List<BoxShadow>? insetShadows;
  final Color? colour;

  const PartStyle({
    this.background,
    this.borderColour,
    this.borderWidth,
    this.radius,
    this.shadows,
    this.insetShadows,
    this.colour,
  });

  static const none = PartStyle();

  /// The same without its edge, for a box whose edge is drawn over its
  /// content instead.
  PartStyle get fill => PartStyle(
        background: background,
        radius: radius,
        shadows: shadows,
        insetShadows: insetShadows,
        colour: colour,
      );

  /// Just its edge, drawn over a box's content so a fill inside can't cover
  /// it where the corner curves. Null when the style sets no edge.
  BoxDecoration? edge(BorderRadius? radius) =>
      borderWidth == null && borderColour == null || borderWidth == 0
          ? null
          : BoxDecoration(
              border: Border.all(
                  color: borderColour ?? const Color(0x00000000),
                  width: borderWidth ?? 1),
              borderRadius: this.radius == null
                  ? radius
                  : BorderRadius.circular(this.radius!),
            );
}

/// Names the part a subtree belongs to, so a rule like `timeline button` can
/// tell a Timeline button from any other.
class StylePart extends InheritedWidget {
  final String name;
  const StylePart({super.key, required this.name, required super.child});

  @override
  bool updateShouldNotify(StylePart old) => old.name != name;
}

/// `effectControls` and `effect-controls` are one name.
String _key(String name) =>
    name.toLowerCase().replaceAll(RegExp('[^a-z0-9]'), '');

const _properties = {
  'background',
  'border',
  'bordercolour',
  'bordercolor',
  'radius',
  'shadow',
  'colour',
  'color',
};

class StyleSheet {
  /// What it is called in Settings: the file's name without its extension.
  final String name;

  /// The style it starts from.
  final ThemeShape base;

  /// The folder the file is in.
  final String folder;

  /// The lines that were skipped, and why, for the Settings row.
  final List<String> problems;

  /// The file's text, to tell whether it changed when it is read again.
  final String source;

  final Map<String, String> _vars;
  final List<_Rule> _rules;
  final Map<String, PartStyle> _resolved = {};
  LumitTheme? _resolvedFor;

  StyleSheet._(this.name, this.base, this.folder, this.problems, this.source,
      this._vars, this._rules);

  /// The style in force, or null for one of the three built in. Set by the
  /// workspace as it builds the theme, the way the icon weight is.
  static StyleSheet? current;

  /// Read a style file's text.
  factory StyleSheet.parse(String text,
      {required String name, String folder = ''}) {
    final problems = <String>[];
    final source = text;
    // Comments become spaces, so every offset still lands on its own line.
    text = text.replaceAllMapped(RegExp(r'/\*[\s\S]*?\*/'),
        (m) => m[0]!.replaceAll(RegExp(r'[^\n]'), ' '));
    int line(int offset) =>
        '\n'.allMatches(text.substring(0, offset)).length + 1;

    var base = ThemeShape.studio;
    text = text.replaceAllMapped(RegExp(r'@base\s+([^;\s]+)\s*;'), (m) {
      final found = ThemeShape.values.asNameMap()[m[1]!.toLowerCase()];
      if (found == null) {
        problems.add(l10n.styleProblem(line(m.start), l10n.styleNoBase(m[1]!)));
      } else {
        base = found;
      }
      return m[0]!.replaceAll(RegExp(r'[^\n]'), ' ');
    });

    final vars = <String, String>{};
    final rules = <_Rule>[];
    for (final block in RegExp(r'([^{}]*)\{([^{}]*)\}').allMatches(text)) {
      final at =
          line(block.start + block[1]!.length - block[1]!.trimLeft().length);
      final props = <String, String>{};
      for (final decl in block[2]!.split(';')) {
        if (decl.trim().isEmpty) continue;
        final colon = decl.indexOf(':');
        if (colon < 0) {
          problems.add(l10n.styleProblem(at, l10n.styleNoColon(decl.trim())));
          continue;
        }
        props[decl.substring(0, colon).trim()] =
            decl.substring(colon + 1).trim();
      }
      for (final raw in block[1]!.split(',')) {
        final sel = raw.trim();
        if (sel == ':root') {
          props.forEach((k, v) {
            if (k.startsWith('--')) {
              vars[_key(k)] = v;
            } else {
              problems.add(l10n.styleProblem(at, l10n.styleRootDashes(k)));
            }
          });
          continue;
        }
        final selector = _readSelector(sel);
        if (selector == null) {
          problems.add(l10n.styleProblem(at, l10n.styleBadSelector(sel)));
          continue;
        }
        final known = <String, String>{};
        props.forEach((k, v) {
          if (_properties.contains(_key(k))) {
            known[_key(k)] = v;
          } else {
            problems.add(l10n.styleProblem(at, l10n.styleNoProperty(k)));
          }
        });
        rules.add(_Rule(selector, known, rules.length));
      }
    }
    // The strongest rule last, so applying them in order lets it win.
    rules.sort((a, b) =>
        a.weight != b.weight ? a.weight - b.weight : a.order - b.order);
    return StyleSheet._(name, base, folder, problems, source, vars, rules);
  }

  static List<_Compound>? _readSelector(String sel) {
    if (sel.isEmpty) return null;
    final out = <_Compound>[];
    final shape = RegExp(r'^([a-z][a-z0-9-]*|\*)?((?:[.:][a-z][a-z0-9-]*)*)$');
    for (final part in sel.toLowerCase().split(RegExp(r'\s+'))) {
      final m = shape.firstMatch(part);
      if (m == null || part.isEmpty) return null;
      final name = m[1] == null || m[1] == '*' ? null : _key(m[1]!);
      final classes = <String>{};
      final states = <String>{};
      for (final s in RegExp(r'([.:])([a-z0-9-]+)').allMatches(m[2]!)) {
        (s[1] == '.' ? classes : states).add(_key(s[2]!));
      }
      out.add(_Compound(name, classes, states));
    }
    return out;
  }

  /// The style's own shape: the base's numbers with its `:root` variables
  /// over them.
  ShapeTokens tokens() {
    final b = ShapeTokens.of(base);
    double num(String k, double fallback) =>
        _number(_vars[k], null) ?? fallback;
    bool flag(String k, bool fallback) => switch (_vars[k]?.trim()) {
          'true' || 'yes' => true,
          'false' || 'no' => false,
          _ => fallback,
        };
    String face(String k, String fallback) {
      final v = _vars[k]?.trim();
      return v == null || v.isEmpty
          ? fallback
          : v.replaceAll(RegExp('["\']'), '');
    }

    return ShapeTokens(
      controlRadius: num('controlradius', b.controlRadius),
      floatRadius: num('floatradius', b.floatRadius),
      cardRadius: num('cardradius', b.cardRadius),
      cardPadding: num('cardpadding', b.cardPadding),
      tileGap: num('tilegap', b.tileGap),
      windowInset: num('windowinset', b.windowInset),
      cardShadow: _vars.containsKey('cardshadow')
          ? _shadows(_vars['cardshadow']!, null).$1
          : b.cardShadow,
      labelCase: LabelCase.values.asNameMap()[_vars['labelcase']?.trim()] ??
          b.labelCase,
      titleCentred: flag('titlecentred', b.titleCentred),
      headerDot: flag('headerdot', b.headerDot),
      strokeWeight: num('strokeweight', b.strokeWeight),
      sectionRadius: num('sectionradius', b.sectionRadius),
      actionRadius: num('actionradius', b.actionRadius),
      wellRadius: num('wellradius', b.wellRadius),
      contentRadius: num('contentradius', b.contentRadius),
      roomed: flag('roomed', b.roomed),
      pillInset: num('pillinset', b.pillInset),
      sansFamily: face('sans', b.sansFamily),
      monoFamily: face('mono', b.monoFamily),
      tabularNumbers: flag('tabularnumbers', b.tabularNumbers),
      kickerTracking: num('kickertracking', b.kickerTracking),
    );
  }

  /// How the style dresses [part] at [context], in [states] (`hover`,
  /// `pressed`, `on`, `disabled`, `focus`) and with [classes] (`primary`).
  /// [inside] names the panel for a part drawn above its panel's
  /// [StylePart], such as the pane itself.
  PartStyle of(
    BuildContext context,
    LumitTheme t,
    String part, {
    Set<String> states = const {},
    Set<String> classes = const {},
    String? inside,
  }) {
    if (_rules.isEmpty) return PartStyle.none;
    final chain = <String>[if (inside != null) _key(inside)];
    context.visitAncestorElements((e) {
      final w = e.widget;
      if (w is StylePart) chain.add(_key(w.name));
      return true;
    });
    final key =
        '${chain.join('/')}|$part|${states.join(',')}|${classes.join(',')}';
    if (!identical(_resolvedFor, t)) {
      _resolved.clear();
      _resolvedFor = t;
    }
    return _resolved.putIfAbsent(
        key, () => _resolve(t, _key(part), chain, states, classes));
  }

  PartStyle _resolve(LumitTheme t, String part, List<String> chain,
      Set<String> states, Set<String> classes) {
    final props = <String, String>{};
    for (final rule in _rules) {
      final last = rule.selector.last;
      if (last.name != null && last.name != part) continue;
      if (!states.containsAll(last.states)) continue;
      if (!classes.containsAll(last.classes)) continue;
      // The rest match the parts this one sits inside, nearest first.
      var at = 0;
      var ok = true;
      for (final want in rule.selector.reversed.skip(1)) {
        while (
            at < chain.length && want.name != null && chain[at] != want.name) {
          at++;
        }
        if (at >= chain.length) {
          ok = false;
          break;
        }
        at++;
      }
      if (ok) props.addAll(rule.props);
    }
    if (props.isEmpty) return PartStyle.none;
    final border = props['border'];
    Color? borderColour;
    double? borderWidth;
    if (border != null) {
      if (border.trim() == 'none') {
        borderWidth = 0;
      } else {
        final words = border.trim().split(RegExp(r'\s+'));
        borderWidth = _number(words.first, null);
        borderColour =
            _colour(borderWidth == null ? border : words.skip(1).join(' '), t);
        borderWidth ??= 1;
      }
    }
    borderColour = _colour(props['bordercolour'] ?? props['bordercolor'], t) ??
        borderColour;
    final shadows =
        props['shadow'] == null ? null : _shadows(props['shadow']!, t);
    return PartStyle(
      background: _colour(props['background'], t),
      borderColour: borderColour,
      borderWidth: borderWidth,
      radius: props['radius']?.trim() == 'round'
          ? ShapeTokens.stadium
          : _number(props['radius'], t),
      shadows: shadows?.$1,
      insetShadows: shadows?.$2,
      colour: _colour(props['colour'] ?? props['color'], t),
    );
  }

  /// A variable's text with any `var(--x)` in it filled in, author's
  /// variables first and then the colour scheme's.
  String? _fill(String? value, LumitTheme? t, [int depth = 0]) {
    if (value == null || depth > 8) return value;
    return value.replaceAllMapped(RegExp(r'var\(\s*--([a-zA-Z0-9-]+)\s*\)'),
        (m) {
      final k = _key(m[1]!);
      final own = _vars[k];
      if (own != null) return _fill(own, t, depth + 1) ?? '';
      if (t != null) {
        for (final token in themeTokens) {
          if (_key(token.key) == k) return _hex(token.read(t));
        }
      }
      return '';
    });
  }

  double? _number(String? value, LumitTheme? t) {
    final v = _fill(value, t)?.trim().replaceFirst(RegExp(r'px$'), '');
    return v == null ? null : double.tryParse(v);
  }

  /// `#rgb`, `#rrggbb` or `#rrggbbaa`, then an optional `60%` that scales the
  /// alpha. `none` is see-through.
  Color? _colour(String? value, LumitTheme? t) {
    final v = _fill(value, t)?.trim();
    if (v == null || v.isEmpty) return null;
    if (v == 'none' || v == 'transparent') return const Color(0x00000000);
    final m =
        RegExp(r'^#([0-9a-fA-F]{3,8})(?:\s+(\d+(?:\.\d+)?)%)?$').firstMatch(v);
    if (m == null) return null;
    var hex = m[1]!;
    if (hex.length == 3) hex = hex.split('').map((c) => '$c$c').join();
    if (hex.length == 6) hex = '${hex}ff';
    if (hex.length != 8) return null;
    final rgba = int.parse(hex, radix: 16);
    var c = Color(((rgba & 0xff) << 24) | (rgba >> 8));
    if (m[2] != null) c = c.withValues(alpha: c.a * double.parse(m[2]!) / 100);
    return c;
  }

  /// `[inset] x y [blur [spread]] colour`, as many as wanted, by commas.
  (List<BoxShadow>, List<BoxShadow>) _shadows(String value, LumitTheme? t) {
    final outer = <BoxShadow>[];
    final inner = <BoxShadow>[];
    final v = _fill(value, t)!.trim();
    if (v == 'none') return (outer, inner);
    for (final one in v.split(',')) {
      final words = one.trim().split(RegExp(r'\s+'));
      final inset = words.remove('inset');
      final colour = _colour(words.isEmpty ? null : words.removeLast(), t);
      final n = [for (final w in words) _number(w, t) ?? 0];
      if (colour == null || n.length < 2) continue;
      final shadow = BoxShadow(
        offset: Offset(n[0], n[1]),
        blurRadius: n.length > 2 ? n[2] : 0,
        spreadRadius: n.length > 3 ? n[3] : 0,
        color: colour,
      );
      (inset ? inner : outer).add(shadow);
    }
    return (outer, inner);
  }

  /// Every style in [folder]: `name.lumstyle` there, or in a folder of its
  /// own beside its pictures and fonts. A missing folder is no styles.
  static List<StyleSheet> readFolder(String folder) {
    final dir = Directory(folder);
    if (!dir.existsSync()) return const [];
    final found = <StyleSheet>[];
    void take(File f) {
      final file = f.uri.pathSegments.last;
      if (!file.toLowerCase().endsWith('.$styleFileExtension')) return;
      try {
        found.add(StyleSheet.parse(f.readAsStringSync(),
            name:
                file.substring(0, file.length - styleFileExtension.length - 1),
            folder: f.parent.path));
      } on FileSystemException {
        // Mid-save, or gone: the next change reads it again.
      }
    }

    for (final e in dir.listSync()) {
      if (e is File) take(e);
      if (e is Directory) {
        for (final inner in e.listSync()) {
          if (inner is File) take(inner);
        }
      }
    }
    found.sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    return found;
  }
}

String _hex(Color c) {
  String two(double v) =>
      (v * 255).round().clamp(0, 255).toRadixString(16).padLeft(2, '0');
  return '#${two(c.r)}${two(c.g)}${two(c.b)}${two(c.a)}';
}

/// How the style in force dresses [part], or nothing when there is none.
PartStyle partStyle(
  BuildContext context,
  LumitTheme t,
  String part, {
  Set<String> states = const {},
  Set<String> classes = const {},
  String? inside,
}) =>
    StyleSheet.current?.of(context, t, part,
        states: states, classes: classes, inside: inside) ??
    PartStyle.none;

/// [d] with the style's say over it: the fill, the edge, the corner and the
/// shadow, each only where the style sets one.
BoxDecoration dress(BoxDecoration d, PartStyle s) {
  if (identical(s, PartStyle.none)) return d;
  var border = d.border;
  if (s.borderWidth != null || s.borderColour != null) {
    final was = border is Border ? border.top : BorderSide.none;
    final side = BorderSide(
      color: s.borderColour ?? was.color,
      width: s.borderWidth ?? (was.width == 0 ? 1 : was.width),
    );
    border = s.borderWidth == 0 ? null : Border.fromBorderSide(side);
  }
  return d.copyWith(
    color: s.background,
    border: border,
    borderRadius: s.radius == null ? null : BorderRadius.circular(s.radius!),
    boxShadow: s.shadows,
  );
}

/// [child] with the style's inset shadows painted over it, inside [radius].
Widget insetShadows(PartStyle s, BorderRadius? radius, Widget child) {
  final shadows = s.insetShadows;
  if (shadows == null || shadows.isEmpty) return child;
  return CustomPaint(
    foregroundPainter:
        _InsetShadowPainter(shadows, radius ?? BorderRadius.zero),
    child: child,
  );
}

class _InsetShadowPainter extends CustomPainter {
  final List<BoxShadow> shadows;
  final BorderRadius radius;
  _InsetShadowPainter(this.shadows, this.radius);

  @override
  void paint(Canvas canvas, Size size) {
    final box = radius.toRRect(Offset.zero & size);
    canvas.save();
    canvas.clipRRect(box);
    for (final s in shadows) {
      // A ring round the box, cast inwards: everything outside the box,
      // shifted by the offset, with its edge blurred.
      final far = s.blurRadius + s.spreadRadius + s.offset.distance + 1;
      final ring = Path()
        ..fillType = PathFillType.evenOdd
        ..addRect(box.outerRect.inflate(far))
        ..addRRect(box.deflate(s.spreadRadius).shift(s.offset));
      canvas.drawPath(
        ring,
        Paint()
          ..color = s.color
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, s.blurSigma),
      );
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_InsetShadowPainter old) =>
      old.shadows != shadows || old.radius != radius;
}
