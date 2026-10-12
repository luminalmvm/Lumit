// How a row says that someone else in a shared project has it in hand: a
// bar of their colour down its leading edge, shared out when it is several
// people, and a tint of the first. The Timeline's property rows and every row
// of the Effect controls panel draw it the same way. And how a row is held
// for someone who is changing a value in it.
//
// None of this is in the tree while the project is not shared. A panel hands
// its rows a [ShareRows] that says so, and the rows come back as they were
// made.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'controls/base.dart' show ThemeScope;
import 'controls/popups.dart' show lumitPopupUp;

/// The bar down a row's leading edge, one band for each person who has the
/// row in hand, and with [tint] a wash of the first person's colour over the
/// whole row.
class TheirBar extends Decoration {
  const TheirBar(this.colours, {this.tint = 0, this.width = 2});

  final List<Color> colours;

  /// How strong the wash over the row is, 0 for none.
  final double tint;
  final double width;

  @override
  BoxPainter createBoxPainter([VoidCallback? onChanged]) =>
      _TheirBarPainter(this);

  @override
  bool operator ==(Object other) =>
      other is TheirBar &&
      other.tint == tint &&
      other.width == width &&
      listEquals(other.colours, colours);

  @override
  int get hashCode => Object.hash(tint, width, Object.hashAll(colours));
}

class _TheirBarPainter extends BoxPainter {
  _TheirBarPainter(this.bar);

  final TheirBar bar;

  @override
  void paint(Canvas canvas, Offset offset, ImageConfiguration configuration) {
    final size = configuration.size;
    if (size == null || bar.colours.isEmpty) return;
    final paint = Paint();
    if (bar.tint > 0) {
      paint.color = bar.colours.first.withValues(alpha: bar.tint);
      canvas.drawRect(offset & size, paint);
    }
    final band = size.height / bar.colours.length;
    for (var i = 0; i < bar.colours.length; i++) {
      paint.color = bar.colours[i];
      canvas.drawRect(
        Rect.fromLTWH(offset.dx, offset.dy + band * i, bar.width, band),
        paint,
      );
    }
  }
}

/// What a panel tells its rows about the shared project: which rows other
/// people have in hand, and where to say that this person has touched one.
///
/// Rows are named by the path the Timeline gives them, so a row taken in
/// hand in one panel is marked in the other. A row with no Timeline path is
/// named by its section and its place in it.
///
/// Always above the rows, and saying nothing while the project is not
/// shared: [active] is false, and a row built under it is the row alone.
class ShareRows extends InheritedWidget {
  const ShareRows({
    super.key,
    required this.marks,
    required this.onTouch,
    required super.child,
  });

  /// By row name, the colour of each other person who has that row in hand.
  final Map<String, List<int>> marks;

  /// Told when this person presses on a row. Null while the project is not
  /// shared.
  final ValueChanged<String>? onTouch;

  bool get active => onTouch != null;

  static ShareRows? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<ShareRows>();

  /// The colours of everyone who has [name] in hand, or anything under it:
  /// what a heading shows for the rows inside it.
  List<int> under(String name) {
    if (marks.isEmpty) return const [];
    final out = <int>[];
    for (final held in marks.entries) {
      if (held.key != name && !held.key.startsWith('$name/')) continue;
      for (final colour in held.value) {
        if (!out.contains(colour)) out.add(colour);
      }
    }
    return out;
  }

  /// [child] as the row called [name]: pressing it says so, and it is drawn
  /// in the colours of whoever else has it. [also] is a second name the same
  /// row answers to, for a row that shows two parameters. While the project
  /// is not shared this is [child] itself.
  Widget row(String name, Widget child, {String? also}) {
    final touched = onTouch;
    if (touched == null) return child;
    final colours = <int>[
      ...?marks[name],
      if (also != null)
        for (final colour in marks[also] ?? const <int>[])
          if (!(marks[name]?.contains(colour) ?? false)) colour,
    ];
    return SharedRow(
      name: name,
      also: also,
      colours: colours,
      onTouch: touched,
      child: child,
    );
  }

  @override
  bool updateShouldNotify(ShareRows old) =>
      active != old.active || !identical(marks, old.marks);
}

/// One named row of a panel in a shared project. Only made while it is
/// shared ([ShareRows.row]).
class SharedRow extends StatelessWidget {
  const SharedRow({
    super.key,
    required this.name,
    this.also,
    required this.colours,
    required this.onTouch,
    required this.child,
  });

  final String name;

  /// A second name the row answers to, when it shows two properties.
  final String? also;
  final List<int> colours;
  final ValueChanged<String> onTouch;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Listener(
      // Watches the press and takes nothing from the row's own controls.
      behavior: HitTestBehavior.translucent,
      onPointerDown: (_) => onTouch(name),
      // The same box whether or not anyone has the row, so a mark arriving
      // does not rebuild the row from nothing under a value being typed.
      child: DecoratedBox(
        position: DecorationPosition.foreground,
        decoration: colours.isEmpty
            ? const BoxDecoration()
            : TheirBar([for (final c in colours) t.personColour(c)], tint: 0.14),
        // The name goes at the head of the row. Its values are at the end,
        // and in a narrow panel the end is cut off.
        child: HeldFor(
            names: [name, if (also != null) also!],
            leading: true,
            child: child),
      ),
    );
  }
}

/// The controls the others in a shared project are changing right now, by
/// the names their rows go by, and for each who has it. Null while the
/// project is not shared.
final ValueNotifier<Map<String, ({int colour, String name})>?> valuesHeld =
    ValueNotifier(null);

/// The rows this person is changing right now, by the names they go by. In a
/// shared project the others are told, and hold those rows for this person
/// until they let go. Said through [sayInHand].
final ValueNotifier<List<String>> valuesInHand = ValueNotifier(const []);

final Map<Object, List<String>> _inHandBy = {};

/// [by] is changing the rows called [names], or with none has let go: a row
/// with a control in use, a handle in the Viewer, keyframes being dragged.
/// A name is a row's Timeline path. A layer's id alone is every row of it.
void sayInHand(Object by, List<String> names) {
  if (listEquals(_inHandBy[by] ?? const <String>[], names)) return;
  names.isEmpty ? _inHandBy.remove(by) : _inHandBy[by] = names;
  valuesInHand.value = [for (final held in _inHandBy.values) ...held];
}

/// Who else is changing the row called [name], if anyone: the row itself,
/// something it is part of, or something inside it.
({int colour, String name})? heldBy(String name) {
  for (final held in (valuesHeld.value ?? const {}).entries) {
    final theirs = held.key;
    if (theirs == name ||
        name.startsWith('$theirs/') ||
        theirs.startsWith('$name/')) {
      return held.value;
    }
  }
  return null;
}

/// Whether someone else is changing any of [names], so that nothing here
/// should start changing them too.
bool heldByOthers(Iterable<String> names) => names.any((n) => heldBy(n) != null);

/// How long a row stays in this person's hand with nothing done to it, as
/// with an editor left open.
const Duration heldQuiet = Duration(seconds: 30);

/// A row's controls in a shared project, under the [names] the row goes by.
///
/// The row is in this person's hand while a pointer drags in it, while
/// something in it is typed in, and while a menu or a picker opened from it
/// is up, whatever the control. The others are told, until it is let go or
/// nothing has been done for [heldQuiet].
///
/// While someone else is changing it the controls are theirs: dimmed,
/// taking no press, with that person's name on the row in their colour,
/// until they let go. Nobody is kept out of a row they already have in hand.
///
/// While the project is not shared this is [child].
class HeldFor extends StatefulWidget {
  const HeldFor(
      {super.key,
      required this.names,
      required this.child,
      this.leading = false});

  final List<String> names;
  final Widget child;

  /// The name goes at the head of the row, and otherwise at its end.
  final bool leading;

  @override
  State<HeldFor> createState() => _HeldForState();
}

class _HeldForState extends State<HeldFor> {
  /// Why the row is in this person's hand.
  bool _drag = false, _keys = false, _menu = false;

  /// Until nothing has been done for [heldQuiet].
  Timer? _quiet;

  /// Hears the keyboard come to anything in the row, and go.
  final FocusNode _within =
      FocusNode(canRequestFocus: false, skipTraversal: true);

  bool get _mine => _drag || _keys || _menu;

  @override
  void initState() {
    super.initState();
    valuesHeld.addListener(_told);
  }

  @override
  void dispose() {
    valuesHeld.removeListener(_told);
    lumitPopupUp.removeListener(_menuGone);
    FocusManager.instance.removeListener(_focused);
    _let(() => _drag = _keys = _menu = false);
    _within.dispose();
    super.dispose();
  }

  /// Who holds what has changed, or sharing has begun or ended.
  void _told() {
    if (valuesHeld.value == null) _let(() => _drag = _keys = _menu = false);
    setState(() {});
  }

  /// Change why the row is in hand, and tell the others. While it is,
  /// anything this person presses or types keeps it.
  void _let(VoidCallback change) {
    final was = _mine;
    change();
    if (_mine && !was) {
      GestureBinding.instance.pointerRouter.addGlobalRoute(_pressed);
      HardwareKeyboard.instance.addHandler(_typed);
    } else if (was && !_mine) {
      GestureBinding.instance.pointerRouter.removeGlobalRoute(_pressed);
      HardwareKeyboard.instance.removeHandler(_typed);
    }
    _mine ? _stir() : _rest();
  }

  void _stir() {
    // A key that shut the row's menu is still on its way round the others
    // who hear keys, this row among them.
    if (!_mine) return;
    _quiet?.cancel();
    // A drag still under way is never quiet.
    _quiet = Timer(heldQuiet, () => _drag ? _stir() : _rest());
    sayInHand(this, widget.names);
  }

  void _rest() {
    _quiet?.cancel();
    sayInHand(this, const []);
  }

  void _pressed(PointerEvent event) {
    if (event is PointerDownEvent) _stir();
  }

  bool _typed(KeyEvent event) {
    _stir();
    return false;
  }

  /// The keyboard has moved. The row has it while something in it is being
  /// typed in. Resting on one of its controls is not that.
  void _focused() => _let(() => _keys = _within.hasFocus &&
      FocusManager.instance.primaryFocus?.context
              ?.findAncestorWidgetOfExactType<EditableText>() !=
          null);

  /// The press is over. A menu or a picker it opened is up by the next
  /// frame, and the row stays in hand until that has gone.
  void _lifted(PointerEvent _) {
    if (_drag) _let(() => _drag = false);
    WidgetsBinding.instance
      ..ensureVisualUpdate()
      ..addPostFrameCallback((_) {
        if (!mounted || _menu || !lumitPopupUp.value) return;
        lumitPopupUp.addListener(_menuGone);
        _let(() => _menu = true);
      });
  }

  void _menuGone() {
    if (lumitPopupUp.value) return;
    lumitPopupUp.removeListener(_menuGone);
    _let(() => _menu = false);
  }

  @override
  Widget build(BuildContext context) {
    if (valuesHeld.value == null) return widget.child;
    final names = widget.names;
    final by = names.any(valuesInHand.value.contains)
        ? null
        : names.map(heldBy).nonNulls.firstOrNull;
    return Listener(
      // A drag, and not a press alone: a press that changes anything is
      // over as it is made.
      onPointerMove: (event) {
        if (!_drag && event.down) _let(() => _drag = true);
      },
      onPointerUp: _lifted,
      onPointerCancel: _lifted,
      child: Focus(
        focusNode: _within,
        onFocusChange: (has) {
          has
              ? FocusManager.instance.addListener(_focused)
              : FocusManager.instance.removeListener(_focused);
          _focused();
        },
        // The same boxes whether or not it is held, so a hold arriving
        // rebuilds nothing under an open menu or a value being typed.
        child: Stack(
          fit: StackFit.passthrough,
          children: [
            AbsorbPointer(
              absorbing: by != null,
              child:
                  Opacity(opacity: by == null ? 1 : 0.5, child: widget.child),
            ),
            if (by != null)
              Positioned(
                key: ValueKey<String>('held-${names.first}'),
                left: widget.leading ? 4 : null,
                right: widget.leading ? null : 0,
                top: 0,
                bottom: 0,
                child: Center(child: _HeldName(by)),
              ),
          ],
        ),
      ),
    );
  }
}

/// The name of whoever is changing something, on their colour.
class _HeldName extends StatelessWidget {
  const _HeldName(this.by);

  final ({int colour, String name}) by;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      constraints: const BoxConstraints(maxWidth: 96),
      padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 2),
      decoration: BoxDecoration(
        color: t.personColour(by.colour),
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
      ),
      // The darkest ground, which every person's colour is bright enough
      // to carry.
      child: Text(by.name,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: t.small.copyWith(
              height: 1, color: t.surface0, fontWeight: FontWeight.w600)),
    );
  }
}

/// Laid over what is not a row, a layer's bar, while someone else in a
/// shared project is changing what is called [names]: it is dimmed, takes
/// no press, and carries that person's name in their colour, as a held row
/// does. Nothing at all while nobody is.
class HeldVeil extends StatelessWidget {
  const HeldVeil({super.key, required this.names});

  final List<String> names;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
        valueListenable: valuesHeld,
        builder: (context, _, __) {
          final by = names.any(valuesInHand.value.contains)
              ? null
              : names.map(heldBy).nonNulls.firstOrNull;
          if (by == null) return const SizedBox.shrink();
          return MouseRegion(
            cursor: SystemMouseCursors.basic,
            child: AbsorbPointer(
              child: ColoredBox(
                key: ValueKey<String>('held-${names.first}'),
                color: ThemeScope.of(context)
                    .theme
                    .surface0
                    .withValues(alpha: 0.5),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Padding(
                    padding: const EdgeInsets.only(left: 4),
                    child: _HeldName(by),
                  ),
                ),
              ),
            ),
          );
        },
      );
}

/// A person in a shared project, drawn small: a disc in their colour with
/// their initial on it, so nobody has to learn who a colour is. The one
/// mark for a person wherever one is drawn, in the strip, beside a
/// composition in the Project panel, and in the people list.
///
/// [stale] draws it fainter, for someone who is only what was last heard.
/// [ringed] puts a ring on it, for the person this machine is following.
/// [letters] is what it carries when that is not the initial alone: two
/// letters for someone whose initial another person here has too, set
/// smaller so the disc stays the size it is.
class PersonDisc extends StatelessWidget {
  final int colour;
  final String name;
  final String? letters;
  final double size;
  final bool stale, ringed;

  const PersonDisc({
    super.key,
    required this.colour,
    required this.name,
    this.letters,
    this.size = personDiscSize,
    this.stale = false,
    this.ringed = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final initial = letters ??
        name.trim().characters.firstOrNull?.toUpperCase() ??
        '';
    final two = initial.characters.length > 1;
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: t.personColour(colour).withValues(alpha: stale ? 0.4 : 1),
        shape: BoxShape.circle,
      ),
      foregroundDecoration: ringed
          ? BoxDecoration(
              shape: BoxShape.circle,
              border: Border.all(color: t.textPrimary, width: 1.5),
            )
          : null,
      child: Text(
        initial,
        style: t.small.copyWith(
          fontSize: size * (two ? 0.5 : 0.62),
          letterSpacing: two ? -0.3 : null,
          height: 1,
          fontWeight: FontWeight.w700,
          color: t.surface0.withValues(alpha: stale ? 0.6 : 1),
        ),
      ),
    );
  }
}

/// How big a person's disc is, and what stands for the people a run of
/// discs has no room for: how many more there are, in the discs' shape.
const double personDiscSize = 14;

class MorePeople extends StatelessWidget {
  final int count;
  const MorePeople({super.key, required this.count});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: personDiscSize,
      constraints: const BoxConstraints(minWidth: personDiscSize),
      alignment: Alignment.center,
      padding: const EdgeInsets.symmetric(horizontal: 3),
      decoration: BoxDecoration(
        color: t.surface3,
        borderRadius: BorderRadius.circular(personDiscSize / 2),
      ),
      child: Text(
        '+$count',
        style: t.small.copyWith(
            fontSize: personDiscSize * 0.62,
            height: 1,
            fontWeight: FontWeight.w700,
            color: t.textSecondary),
      ),
    );
  }
}

/// A run of [people] is drawn whole up to this many marks. Past it the last
/// mark is how many more there are.
// ponytail: a fixed count, not what the place has room for. Measure the room
// if a strip or a row is ever found with space going spare.
int peopleDrawn(int people, int marks) => people <= marks ? people : marks - 1;
