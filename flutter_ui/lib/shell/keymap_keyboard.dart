// The keyboard picture on Settings → Shortcuts: every key, lit when the keymap
// gives it something to do with the modifiers chosen.
//
// It draws what KeymapState already holds and decides nothing. Each cap takes
// its name and its print from the keyboard layout (state/key_layout.dart).

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/key_layout.dart';
import '../state/keymap.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'settings_rows.dart';

/// One cap: the keymap's name for the key, and its width in key units. A null
/// name is a gap, an empty one a key no shortcut can use.
typedef _Cap = (String?, double);

List<_Cap> _caps(String keys, [double width = 1]) =>
    [for (final key in keys.split(' ')) (key, width)];

const double _nav = 1.25;

/// The board: the main block, then the navigation keys.
List<List<_Cap>> _rows(bool iso) => [
  [
    ('Escape', 1.5),
    (null, 0.5),
    ..._caps('F1 F2 F3 F4'),
    (null, 0.5),
    ..._caps('F5 F6 F7 F8'),
    (null, 0.5),
    ..._caps('F9 F10 F11 F12'),
  ],
  [
    ..._caps('` 1 2 3 4 5 6 7 8 9 0 - ='),
    ('Backspace', 2),
    (null, 0.5),
    ..._caps('Insert Home PageUp', _nav),
  ],
  [
    ('Tab', 1.5),
    ..._caps('Q W E R T Y U I O P [ ]'),
    iso ? (null, 1.5) : ('\\', 1.5),
    (null, 0.5),
    ..._caps('Delete End PageDown', _nav),
  ],
  [
    ('', 1.75),
    ..._caps("A S D F G H J K L ; '"),
    if (iso) ('\\', 1),
    iso ? (null, 1.25) : ('Enter', 2.25),
  ],
  [
    ('Shift', iso ? 1.25 : 2.25),
    if (iso) ('IntlBackslash', 1),
    ..._caps('Z X C V B N M , . /'),
    ('Shift', 2.75),
    (null, 0.5 + _nav),
    ('ArrowUp', _nav),
  ],
  [
    ('Mod', 1.5),
    ('', 1),
    ('Alt', 1.5),
    ('Space', 7),
    ('Alt', 1.5),
    ('', 1),
    ('Mod', 1.5),
    (null, 0.5),
    ..._caps('ArrowLeft ArrowDown ArrowRight', _nav),
  ],
];

const double _boardUnits = 15.5 + 3 * _nav;

Path _isoEnter(Rect rect, double notch, double step, double radius) {
  final points = [
    rect.topLeft,
    rect.topRight,
    rect.bottomRight,
    Offset(rect.left + notch, rect.bottom),
    Offset(rect.left + notch, rect.top + step),
    Offset(rect.left, rect.top + step),
  ];
  final path = Path();
  for (var i = 0; i < points.length; i++) {
    final at = points[i];
    final before = points[(i + points.length - 1) % points.length];
    final after = points[(i + 1) % points.length];
    Offset toward(Offset p) {
      final d = p - at;
      return at + d / d.distance * math.min(radius, d.distance / 2);
    }

    final start = toward(before);
    i == 0 ? path.moveTo(start.dx, start.dy) : path.lineTo(start.dx, start.dy);
    final end = toward(after);
    path.quadraticBezierTo(at.dx, at.dy, end.dx, end.dy);
  }
  return path..close();
}

class _IsoEnterBorder extends ShapeBorder {
  final Color edge;
  final double notch;
  final double step;
  final double radius;

  const _IsoEnterBorder(this.edge, this.notch, this.step, this.radius);

  @override
  EdgeInsetsGeometry get dimensions => EdgeInsets.zero;

  @override
  Path getInnerPath(Rect rect, {TextDirection? textDirection}) =>
      getOuterPath(rect);

  @override
  Path getOuterPath(Rect rect, {TextDirection? textDirection}) =>
      _isoEnter(rect.deflate(0.5), notch, step, radius);

  @override
  void paint(Canvas canvas, Rect rect, {TextDirection? textDirection}) =>
      canvas.drawPath(
          getOuterPath(rect),
          Paint()
            ..style = PaintingStyle.stroke
            ..color = edge);

  @override
  ShapeBorder scale(double t) => this;
}

const double _largestKey = 34;

const _modifiers = {'Mod', 'Alt', 'Shift'};

/// The contexts that are one panel. Tools and Panels are asked from every
/// panel, so a key bound there works anywhere.
const _onePanel = {
  BridgeKeyContext.project,
  BridgeKeyContext.timeline,
  BridgeKeyContext.viewer,
  BridgeKeyContext.graph,
  BridgeKeyContext.nodes,
  BridgeKeyContext.effects,
  BridgeKeyContext.cut,
};

/// What is printed on a cap. A modifier reads the way a chord spells it here,
/// so it is Ctrl on Windows and ⌘ on a Mac.
String _legend(String key) {
  if (_modifiers.contains(key)) {
    final spelt = chordLabel('$key+');
    return spelt.endsWith('+') ? spelt.substring(0, spelt.length - 1) : spelt;
  }
  return switch (key) {
    'Space' => '',
    // Already on the number pad here, so the star needs no saying so.
    '*' => '*',
    // On a key cap the mark is the key, where a menu spells it out.
    _ when keyLayout.legend(key) == '`' => '`',
    _ => chordLabel(key).replaceFirst(' ', '\n'),
  };
}

String _layoutName(String id) => switch (id) {
      'us' => l10n.keymapLayoutUs,
      'uk' => l10n.keymapLayoutUk,
      'de' => l10n.keymapLayoutGerman,
      'fr' => l10n.keymapLayoutFrench,
      'es' => l10n.keymapLayoutSpanish,
      'it' => l10n.keymapLayoutItalian,
      'sv' => l10n.keymapLayoutSwedish,
      'de-ch' => l10n.keymapLayoutSwissGerman,
      'ru' => l10n.keymapLayoutRussian,
      _ => l10n.keymapLayoutAutomatic,
    };

class KeymapKeyboard extends StatefulWidget {
  final KeymapState keymap;

  const KeymapKeyboard({super.key, required this.keymap});

  @override
  State<KeymapKeyboard> createState() => _KeymapKeyboardState();
}

class _KeymapKeyboardState extends State<KeymapKeyboard> {
  final Set<String> _held = {};

  String? _hover;

  /// The chord [key] makes with the modifiers clicked on, spelt the way the
  /// engine spells one.
  String _chord(String key) => [
        for (final modifier in _modifiers)
          if (_held.contains(modifier)) modifier,
        key,
      ].join('+');

  Widget _cap(
      LumitTheme t, String place, double width, double unit, Set<String> drawn,
      {double rows = 1}) {
    final key = keyLayout.nameFor(place);
    final modifier = _modifiers.contains(key);
    final bindings = key.isEmpty || modifier
        ? const <BridgeKeyBinding>[]
        : widget.keymap.bindingsFor(_chord(key));
    final anywhere = bindings.any((b) => !_onePanel.contains(b.context));
    final onePanel = bindings.any((b) => _onePanel.contains(b.context));
    final on = modifier && _held.contains(key);
    final hovered = key.isNotEmpty && _hover == key;

    // A key with nothing on it is an outline, so the taken ones stand out.
    final fill = on
        ? t.accent
        : anywhere
            ? t.surface4
            : null;
    final edge = onePanel
        ? t.accent
        : hovered
            ? t.hairlineStrong
            : t.hairline;
    final ink = on
        ? accentInk(t)
        : bindings.isNotEmpty || hovered
            ? t.textPrimary
            : t.textMuted;

    Widget cap = Container(
      margin: const EdgeInsets.all(1),
      alignment: rows > 1 ? const Alignment(0, -0.5) : Alignment.center,
      decoration: rows > 1
          ? ShapeDecoration(
              color: fill,
              shape: _IsoEnterBorder(
                  edge, unit / 4, unit - 2, t.tokens.controlRadius))
          : BoxDecoration(
              color: fill,
              border: Border.all(color: edge),
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
      child: FittedBox(
        fit: BoxFit.scaleDown,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 2),
          child: Text(
            _legend(key),
            textAlign: TextAlign.center,
            style: t.caption.copyWith(color: ink, height: 1.1),
          ),
        ),
      ),
    );
    if (modifier) {
      cap = MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = key),
        onExit: (_) {
          if (_hover == key) setState(() => _hover = null);
        },
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: () => setState(() {
            if (!_held.remove(key)) _held.add(key);
          }),
          child: cap,
        ),
      );
    } else if (key.isNotEmpty) {
      cap = MouseRegion(
        onEnter: (_) => setState(() => _hover = key),
        onExit: (_) {
          if (_hover == key) setState(() => _hover = null);
        },
        child: cap,
      );
    }
    return SizedBox(
      // Shift, Alt and Ctrl are drawn twice, and one name can't key two caps.
      key: drawn.add(key) ? ValueKey('keymap-key-$key') : null,
      width: width * unit,
      height: rows * unit,
      child: cap,
    );
  }

  Widget _readout(LumitTheme t) {
    final key = _hover;
    // A modifier lights under the pointer but is not a chord to read out.
    if (key == null || _modifiers.contains(key)) {
      return const SizedBox.shrink();
    }
    final chord = _chord(key);
    final bindings = widget.keymap.bindingsFor(chord);
    final where = {
      for (final group in widget.keymap.groups) group.context: group.label,
    };
    return Wrap(
      key: const ValueKey('keymap-key-readout'),
      spacing: 12,
      runSpacing: 2,
      children: [
        Text(chordLabel(chord), style: t.small.copyWith(color: t.textPrimary)),
        if (bindings.isEmpty) Text(l10n.keymapNotSet, style: t.small),
        for (final binding in bindings)
          Text.rich(
            TextSpan(
              text: '${engineLabel(where[binding.context] ?? '')}  ',
              children: [
                TextSpan(
                  text: engineLabel(binding.description),
                  style: TextStyle(color: t.textSecondary),
                ),
              ],
            ),
            style: t.small,
          ),
      ],
    );
  }

  Widget _swatch(LumitTheme t, Color? fill, Color edge, String label) => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 9,
            height: 9,
            decoration: BoxDecoration(
              color: fill,
              border: Border.all(color: edge),
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
          ),
          const SizedBox(width: 5),
          Text(label, style: t.small),
        ],
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: settingsRowPadding),
      child: LayoutBuilder(builder: (context, constraints) {
        final unit = math.min(constraints.maxWidth / _boardUnits, _largestKey);
        final drawn = <String>{};
        final iso = keyLayout.iso;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Stack(
              children: [
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    for (final row in _rows(iso))
                      Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          for (final (place, width) in row)
                            place == null
                                ? SizedBox(width: width * unit, height: unit)
                                : _cap(t, place, width, unit, drawn),
                        ],
                      ),
                  ],
                ),
                if (iso)
                  Positioned(
                    left: 13.5 * unit,
                    top: 2 * unit,
                    child: _cap(t, 'Enter', 1.5, unit, drawn, rows: 2),
                  ),
              ],
            ),
            const SizedBox(height: 6),
            ConstrainedBox(
              constraints: const BoxConstraints(minHeight: settingsRowHeight),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Expanded(child: _readout(t)),
                  const SizedBox(width: 12),
                  _swatch(t, t.surface4, t.hairline, l10n.keyAnywhere),
                  const SizedBox(width: 10),
                  _swatch(t, null, t.accent, l10n.keymapKeyboardPanel),
                  const SizedBox(width: 16),
                  Text(l10n.keymapLayout, style: t.small),
                  const SizedBox(width: 6),
                  SizedBox(
                    width: 120,
                    height: settingsControlHeight,
                    child: BareDropdown<String>(
                      key: const ValueKey('keymap-layout'),
                      value: keyLayout.override ?? '',
                      options: ['', ...keyLayouts.keys],
                      label: _layoutName,
                      onChanged: (id) =>
                          widget.keymap.setKeyLayout(id.isEmpty ? null : id),
                    ),
                  ),
                ],
              ),
            ),
          ],
        );
      }),
    );
  }
}
