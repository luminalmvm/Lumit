// The keyboard picture on Settings → Shortcuts: every key, lit when the keymap
// gives it something to do with the modifiers chosen.
//
// It draws what KeymapState already holds and decides nothing. The layout is
// the US one, since the keymap names a key by what is printed on it.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/keymap.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'settings_rows.dart';

/// One cap: the keymap's name for the key, and its width in key units. A null
/// name is a gap, an empty one a key no shortcut can use.
typedef _Cap = (String?, double);

List<_Cap> _caps(String keys, [double width = 1]) =>
    [for (final key in keys.split(' ')) (key, width)];

/// The board, twenty units wide: the main block, then the navigation keys.
final List<List<_Cap>> _rows = [
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
    ..._caps('Insert Home PageUp', 1.5),
  ],
  [
    ('Tab', 1.5),
    ..._caps('Q W E R T Y U I O P [ ]'),
    ('\\', 1.5),
    (null, 0.5),
    ..._caps('Delete End PageDown', 1.5),
  ],
  [
    ('', 1.75),
    ..._caps("A S D F G H J K L ; '"),
    ('Enter', 2.25),
  ],
  [
    ('Shift', 2.25),
    ..._caps('Z X C V B N M , . /'),
    ('Shift', 2.75),
    (null, 2),
    ('ArrowUp', 1.5),
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
    ..._caps('ArrowLeft ArrowDown ArrowRight', 1.5),
  ],
];

const double _boardUnits = 20;

/// The widest a one-unit key is drawn, however wide the window gets.
const double _largestKey = 34;

const _modifiers = {'Mod', 'Alt', 'Shift'};

/// The contexts that are one panel. Tools and Panels are asked from every
/// panel, so a key bound there works anywhere.
const _onePanel = {
  BridgeKeyContext.project,
  BridgeKeyContext.timeline,
  BridgeKeyContext.viewer,
  BridgeKeyContext.graph,
  BridgeKeyContext.effects,
};

/// What is printed on a cap. A modifier reads the way a chord spells it here,
/// so it is Ctrl on Windows and ⌘ on a Mac.
String _legend(String key) {
  if (_modifiers.contains(key)) {
    final spelt = chordLabel('$key+');
    return spelt.endsWith('+') ? spelt.substring(0, spelt.length - 1) : spelt;
  }
  return switch (key) {
    // A real space bar is blank.
    'Space' => '',
    'ArrowUp' => '↑',
    'ArrowDown' => '↓',
    'ArrowLeft' => '←',
    'ArrowRight' => '→',
    _ => key.replaceFirst('Page', 'Page\n'),
  };
}

class KeymapKeyboard extends StatefulWidget {
  final KeymapState keymap;

  const KeymapKeyboard({super.key, required this.keymap});

  @override
  State<KeymapKeyboard> createState() => _KeymapKeyboardState();
}

class _KeymapKeyboardState extends State<KeymapKeyboard> {
  /// The modifiers clicked on, which pick the layer the board shows.
  final Set<String> _held = {};

  /// The key under the pointer.
  String? _hover;

  /// The chord [key] makes with the modifiers clicked on, spelt the way the
  /// engine spells one.
  String _chord(String key) => [
        for (final modifier in _modifiers)
          if (_held.contains(modifier)) modifier,
        key,
      ].join('+');

  Widget _cap(LumitTheme t, String key, double width, double unit, bool keyed) {
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
      alignment: Alignment.center,
      decoration: BoxDecoration(
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
      key: keyed ? ValueKey('keymap-key-$key') : null,
      width: width * unit,
      height: unit,
      child: cap,
    );
  }

  /// What the key under the pointer does.
  Widget _readout(LumitTheme t) {
    final key = _hover;
    if (key == null) return const SizedBox.shrink();
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
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final row in _rows)
              Row(
                mainAxisSize: MainAxisSize.min,
                children: () {
                  final drawn = <String>{};
                  return [
                    for (final (key, width) in row)
                      key == null
                          ? SizedBox(width: width * unit, height: unit)
                          : _cap(t, key, width, unit, drawn.add(key)),
                  ];
                }(),
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
                ],
              ),
            ),
          ],
        );
      }),
    );
  }
}
